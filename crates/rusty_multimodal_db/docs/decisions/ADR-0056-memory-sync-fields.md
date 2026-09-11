# ADR-0056: `deleted_at` and `node_id` on `Memory` — sentinels for null, a schema-tag bump for the layout

- Status: **Accepted** (2026-09-07 — the owner's pick, option (a): N1
  sentinels plus L1 a schema-tag bump with a distinct failure and no
  upgrade; (b), (c), and (d) declined. Proposed design-only in PR #216,
  since it changes `Memory`'s on-disk layout after `ADR-0053`;
  implemented on the following branch as `SERVER-001` v0.46.0 / FR-056.)
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-MEMORY-SYNC-FIELDS-DESIGN.md` (the full
  design), `ADR-0048` (the projection), `ADR-0053` (durable
  directories), `STORAGE-015` (the record blob), `ADR-0055` (`Page`),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 3).
- Supersedes/Superseded by: none proposed.

## Context

The hub keeps a soft-delete timestamp and a writing node per memory,
both nullable; without them three `HubStore` methods cannot be honest.
The projection has no null, and its record layout is unversioned: a
`bincode` body written with twelve fields and read with fourteen
mis-decodes rather than failing. Durable directories exist since
today.

## Decision

Add `deleted_at_unix_ms: i64` and `node_id: String` to `Memory` with
documented sentinels — `0` is live, `""` is unattributed — as wire
fields 11 and 12; no protocol change. Bump `Memory`'s schema tag to
`memory::Memory@2` so a layout-1 directory fails distinctly on open,
never mis-decodes and is never recreated; `memory_server` names the
remedy (re-push from the consumer, the source of truth). No upgrade
tool this round: no deployment holds a layout-1 directory, and a sync
target is rebuilt by a full push. A layout version with an in-place
upgrade (design option L2) is the named follow-up, triggered by the
first directory that cannot be re-pushed. The server attaches no
meaning to `deleted_at`; `Delete` stays the hard delete.

## Consequences

- Positive: `compact_tombstones`, `stats.tombstones`, and
  `count_by_origin_node` become real on the spike adapter with `Query`,
  `Aggregate`, and `Page` as they stand.
- Positive: no wire, client, or fixture change.
- Named, not hidden: a sentinel is a convention the caller must know;
  the field docs and schema say it.
- Named, not hidden: an old directory is refused, not upgraded; the
  remedy is a re-push.

## Considered options

Null: **(N1) sentinels** — recommended; **(N2) a nullable `ScanValue`**
at a protocol bump — the general answer, deferred until a column has
no lossless sentinel; **(N3) presence fields** — pairs that can
disagree. Layout: **(L1) tag bump, distinct failure, no upgrade** —
recommended now; **(L2) a layout version on the record type with an
in-place upgrade** — the principled follow-up; **(L3) tolerant
decoding** — impossible with `bincode`; **(L4) tag bump and rewrite
every companion** — five files for what (L2) does with two.

The owner's shorthand: **(a)** N1 + L1 as proposed; **(b)** N1 + L2
now; **(c)** N2 with either layout option; **(d)** decline — the
projection stays as it is and tombstones remain the consumer's.

## Acceptance and implementation

- 2026-09-07: proposed, design only (PR #216).
- 2026-09-07: the owner picked option (a).
- 2026-09-07: implemented as `SERVER-001` v0.46.0 / FR-056 —
  `src/generic/memory.rs` (two fields, `SCHEMA_TAG` `memory::Memory@2`),
  `src/server/memory.rs` (tags 11/12, `fields_of`, `memory_from_fields`
  with `deleted_at_unix_ms >= 0`, `describe`, `READ_ONLY_FIELDS`),
  `src/bin/memory_server.rs` (sample data), every test spelling out a
  `Memory`; tests: `generic/memory` +1 (a twelve-field-layout blob
  refused distinctly, nothing rewritten or recreated),
  `server_memory_integration` +1 (sentinels, the purge set, per-node
  counts, a negative stamp refused, a restart); no wire, client, or
  fixture change, `PROTOCOL_VERSION` stays 20.
