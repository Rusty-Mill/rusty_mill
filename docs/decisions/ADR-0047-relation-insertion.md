# ADR-0047: Runtime relation insertion — `Link`/`MultiLink`, the edge log, open labels, `Request::Link` at protocol 14

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-06 — the owner's "Accept as designed", option (a): open
  labels with a manifest, symmetric edges, insert-or-ignore on the
  wire; (b), (c), and (d) declined. Recorded in "Acceptance and
  implementation" below.) Proposed and implemented on one branch under
  the owner's standing mandate and the `ADR-0011`/`ADR-0046`
  precedent; every piece additive and reversible.
- Date: 2026-09-06
- Deciders: baileyrd
- Related: `docs/design/SERVER-LINK-DESIGN.md` (the full design),
  `ADR-0046` (records became insertable; edges did not — this closes
  that), `ADR-0042` F3/F4/F10 (open labels are the consumer's real
  model; direction and metadata are not copied; insert-or-ignore is),
  `ADR-0039`/`FR-041` (`MultiSymmetric`'s fixed-label construction,
  opened here), `STORAGE-016`/`ADR-0018` (the edge blob), `ADR-0044`
  (`Join`/`DescribeRelations`, which see new labels for free).
- Supersedes/Superseded by: none. Appends one `Request` variant, two
  `crate::generic` traits, one error/outcome pair, one
  `ConnectionStore` method with a default, and two companion file
  kinds (edge logs, a label manifest) that appear only when used.

## Context

After `ADR-0046` a running store can gain records but not edges: both
relation layers build their adjacency once, from a caller-supplied
list, and keep no path to write to afterwards. An `Entity` inserted at
runtime is an island. The consumer records an edge whenever a memory's
SPO triple names two known entities, under a free-form label,
insert-or-ignore (`entity.rs:770-826`). `ADR-0042` named the
fixed-label model — not the placeholder strings — as the mismatch, and
open-label edges at write time as "a design round of its own".

## Decision

Adopt `docs/design/SERVER-LINK-DESIGN.md`:

1. **`Link<R, Marker>` / `MultiLink<R>`** with `LinkOutcome::{Linked,
   AlreadyLinked}` and `LinkError::{UnknownRecord, SelfLoop,
   InvalidLabel, Durability}`. `Symmetric::link` and `MultiSymmetric::
   link` check both records exist, refuse a self-loop, answer
   `AlreadyLinked` for a present edge with nothing written, and
   otherwise append the pair to an **edge log** beside the edge blob
   (the insert log's format over `(Id, Id)`) before updating adjacency.
   Folded into the blob at the next `open`, as the record log is.
2. **Open labels.** `MultiSymmetric::link` under a label it has no
   adjacency for creates it: an empty blob and a rewritten **label
   manifest** (`<path>.relations`) first, then the edge. `open`/
   `open_portable` union the manifest into the caller's labels, so a
   portable reopen discovers runtime labels. A label is validated to a
   filesystem-safe charset (1–64 bytes of `[A-Za-z0-9_-]`, not leading
   `-`) at the adapter and again at the layer.
3. **`ConnectionStore::link_records`** with a default of `Unsupported`;
   `Entity` (open labels) and `Employee` (`research`; its one fixed
   label) implement it.
4. **`Request::Link { left, right, relation }`** at 22, protocol 14,
   answered `Ok` for a new *or* an existing edge (the consumer's
   insert-or-ignore); `RecordNotFound`/`Malformed`/`Storage` otherwise;
   `Malformed` below 14, `Unauthorized` for `ReadOnly`, `SessionOpen`
   in a session. No new `ErrorCode`.
5. **`SchemaDrivenClient::link`** and the Python client's `Client.
   link`; the Rust client re-fetches its relation list after linking
   under a label it had not seen.

## Consequences

- Positive: the consumer's `upsert_entity_relation` has a backend path
  with its own semantics (open labels, idempotent), and every read —
  `Neighbors`, `NeighborsByRelation`, `ListRelationKinds`, `Join`,
  `traverse` — sees a new edge and a new label with no change of its
  own.
- Positive: no existing file format changes; a store never linked in
  has no edge log, and a `MultiSymmetric` directory gains its manifest
  on first open by this version, which every older reader ignores.
- Cost, named: a label is a file-name fragment, so it is restricted to
  a safe charset; the consumer's space-bearing predicates need a bridge
  mapping (`works with` → `works_with`). Stated as a non-goal, not
  discovered later.
- Cost, named: edges are symmetric and carry no metadata; the
  consumer's direction and timestamps are not represented. A directed
  kind is its own round.
- Cost, named: edge logs, like the record log, grow until the next
  open; the same revisit trigger.
- Named, not hidden: the implementation landed before acceptance, as
  `ADR-0046`'s did; its file list is in `SERVER-001-FR-047`.

## Considered options

- **(a) (proposed, accepted)** The design as written: open labels
  with a manifest, symmetric edges, insert-or-ignore on the wire.
- **(b)** Fixed labels only — no manifest, no label creation; `link`
  under an unknown label is `Malformed`. Cheaper; leaves the mismatch
  `ADR-0042` identified in place.
- **(c)** Store layer only, no wire — rejected for `ADR-0046`'s reason.
- **(d)** Revert.

## Acceptance and implementation

- Options offered at proposal: **(a) accept as designed**; **(b)**
  accept with fixed labels only (no label creation, no manifest);
  **(c)** store layer only, no wire; **(d)** decline.
- 2026-09-06: proposed. Implementation follows as `SERVER-001-FR-047`
  (v0.37.0) on the same branch.
- 2026-09-06: implemented as `SERVER-001-FR-047` (v0.37.0), landed as
  designed with no deviation — see that requirement's entry and
  `docs/PROJECT-STATUS.md` item 137.
- 2026-09-06: **accepted as designed** (option (a)) — the owner's
  "Accept as designed" on PR #199, on this session's recommendation
  (open labels are the model the consumer actually has; the manifest
  is one small file). (b), (c), (d) declined. No change to the
  implementation.
