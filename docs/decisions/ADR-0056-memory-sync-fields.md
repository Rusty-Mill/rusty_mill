# ADR-0056: `deleted_at` and `node_id` on `Memory` — sentinels for null, a schema-tag bump for the layout

- Status: **Proposed, design only** (2026-09-07). Not implemented: it
  changes `Memory`'s on-disk record layout after `ADR-0053` made
  directories durable, and the working agreement asks before a schema
  or data migration. The owner picks from "Considered options" below;
  the implementation round follows the pick.
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

## Decision (proposed)

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

## Consequences (if accepted)

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

- 2026-09-07: proposed, design only. Awaiting the owner's option.
