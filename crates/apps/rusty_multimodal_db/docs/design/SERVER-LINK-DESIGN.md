# Server Relation Insertion — `Link`, the edge log, open labels (Proposed)

- Status: **Accepted** (2026-09-06, option (a), as designed — the
  owner's "Accept as designed" on PR #199). Proposed and implemented
  on the same branch under the owner's standing mandate for this line
  ("start the relation insertion round"), the `ADR-0046` cadence —
  design first, then the code in its own commit, every piece additive
  and reversible.
- Date: 2026-09-06
- Related: `ADR-0046`/`docs/design/SERVER-INSERT-DESIGN.md` (the
  round this completes — an inserted `Entity` had no edges and no way
  to gain any; the insert log this round generalizes), `ADR-0042`
  finding F3 ("there is no relation vocabulary … open-label edges
  created at write time would be a redesign of `MultiSymmetric`'s
  construction model and of `Request::ListRelationKinds` … a design
  round of its own" — this is that round) and F4/F10 (edges are
  directed triples with timestamps, insert-or-ignore, convergent ids —
  what this round does *not* copy, and why), `ADR-0039`/`FR-041`
  (`MultiSymmetric`, the fixed-label model this round opens),
  `STORAGE-016`/`ADR-0018` (the edge blob the edge log sits beside),
  `ADR-0044` (`Join` and `DescribeRelations`, which see a new label
  through the adapter's own `list_relation_kinds`).

## Purpose and scope

`ADR-0046` made records insertable. It left the graph read-only: a
`Symmetric` or `MultiSymmetric` layer builds its adjacency once, at
construction, from an edge list the caller hands it, and nothing on
the wire or in `crate::generic::query` adds an edge afterwards. So an
`Entity` inserted at runtime is an island, and `Request::Neighbors`,
`NeighborsByRelation`, `Join`, and `traverse` never see it connected.

The consumer's edge-writing path (`rusty_remind_me` at `29602f1`,
`entity.rs:770-826`): `upsert_entity_relation(subject, relation,
object)` — insert-or-ignore on a derived id, the label's whitespace
collapsed, called from `maybe_link_entity_relation` whenever a memory's
free-text SPO triple names two known entities. Two properties matter
here and were named by `ADR-0042`: **labels are free-form per triple**
(F3), and **re-recording an edge is a no-op, never an error** (F10).

This document proposes runtime relation insertion, end to end:

1. `Link<R, Marker>` (one relation) and `MultiLink<R>` (labeled
   relations) in `crate::generic::query`, implemented by `Symmetric`
   and `MultiSymmetric` over an **edge log** beside each edge blob —
   the insert log's format, reused — folded at the next open exactly
   as `ADR-0046`'s record log is;
2. **open labels**: `MultiSymmetric` gains a relation at first use,
   recorded in a small **label manifest** so a portable reopen finds
   it without the caller naming it — the redesign F3 called for;
3. `ConnectionStore::link_records` with a default of `Unsupported`,
   implemented by `Entity` (open labels) and, `research`-gated, by
   `Employee` (its one fixed label — the `Symmetric` path proven over
   the wire);
4. `Request::Link { left, right, relation }` at protocol 14, answered
   `Ok` whether the edge is new or already present (the consumer's
   insert-or-ignore), with the insert round's gates;
5. `SchemaDrivenClient::link` and the Python client's `Client.link`.

## Non-goals

- **Not directed edges.** `Symmetric`/`MultiSymmetric` are symmetric
  by construction and `Neighbors` returns bare ids; the consumer's
  subject/object direction (F4) is not representable today and this
  round does not add a directed relation kind. A caller that needs
  direction encodes it in the label (`manages`/`managed_by`), which
  the open-label model now allows.
- **Not edge metadata.** No timestamps, no per-edge id, no weight
  (F4: real edges carry `created_at`/`updated_at`/`node_id`, no
  weight). An edge is a pair under a label.
- **Not unlinking.** "No runtime deletion" stands for edges as for
  records.
- **Not cross-table links.** The consumer's `memory_entities` joins a
  memory to an entity — two tables; this crate's connection serves one
  (`ADR-0045`). The `Memory` domain round owns that.
- **Not parent/child edges.** A `ChildOf` relation is a *field* of the
  child record (`Order::customer_id`), set at insert; `Reversed` indexes
  it there. Nothing to add.
- **Not label normalization beyond validation.** The consumer collapses
  whitespace and keeps case; its labels may contain spaces. This crate
  derives a *file name* from a label (`<path>.<label>.edges`), so a
  label is validated at the boundary to a filesystem-safe charset
  (below) and otherwise stored as given. Mapping `"works with"` to
  `works_with` is the bridge's job, stated here so it is nobody's
  surprise.
- **Not `Dog`, `Reminder`, `Order`.** `Dog`'s `ProductionStore` is the
  bespoke store; `Reminder`/`Order` have no symmetric relation. All
  keep the trait's default.

## Context and terminology

`Symmetric<S, R, Marker>` holds `adjacency: HashMap<Id, Vec<Id>>` built
by `new(inner, edges)`; `create`/`open`/`open_portable` add a
companion edge blob at a caller-supplied `edges_path` (`STORAGE-016`),
one `bincode` `Vec<(Id, Id)>` image rewritten whole when the caller's
edge list differs. `MultiSymmetric<S, R>` holds `adjacency:
HashMap<String, HashMap<Id, Vec<Id>>>` from a `[(label, edges)]` list;
one blob per label at `<path>.<label>.edges`; `open_portable(inner,
path, labels)` needs the labels named — "an open-ended,
self-discovering label set would need a manifest file this primitive
does not have (a real, named limitation)" (its own doc comment).

Neither layer keeps its path after construction, so neither can write
after it. Both build from a caller-supplied list at reopen — the same
caller-list contract `ADR-0046` narrowed for records.

- **Edge log** — `<blob path>.inserts` beside an edge blob: the insert
  log's format over `(Id, Id)` items, tagged with `R::SCHEMA_TAG`.
- **Label manifest** — `<path>.relations`: the set of labels a
  `MultiSymmetric` has ever had an edge blob for, tagged, so a portable
  reopen discovers runtime-created labels.
- **Valid label** — 1 to 64 bytes of `[A-Za-z0-9_-]`, not starting
  with `-`. Refuses `..`, `/`, whitespace, and anything else that
  could steer a file name.

## Requirements

- `LNK-FR-001` **Traits and errors.** `query::Link<R: SymmetricRelation
  <Marker>, Marker>` with `fn link(&mut self, a: R::Id, b: R::Id) ->
  Result<LinkOutcome, LinkError<R::Id>>`; `query::MultiLink<R: Record>`
  with `fn link(&mut self, relation: &str, a, b) -> …`. `LinkOutcome::
  {Linked, AlreadyLinked}`. `LinkError<Id>::{UnknownRecord(Id),
  SelfLoop(Id), InvalidLabel(String), Durability(DurabilityError)}` in
  `crate::generic`. `pub fn valid_relation_label(&str) -> bool` in
  `crate::generic::store`.
- `LNK-FR-002` **`Symmetric::link`.** Both ids must have a record
  (`S: GetById<R>`; `UnknownRecord` names the first missing one, `a`
  checked before `b`); `a == b` is `SelfLoop`; an edge already present
  in either orientation is `AlreadyLinked` with nothing written; else,
  when the layer has an edge path, the pair is appended to the edge log
  and `sync_data`ed before the adjacency gains both directions. A layer
  built with `new` has no path and links in memory only.
- `LNK-FR-003` **The edge log** reuses `insert_log`'s format over an
  explicit tag: `GENINSL\0`, version 1, `R::SCHEMA_TAG`'s hash,
  `u32`-length-prefixed `bincode` `(Id, Id)` entries, torn tail dropped,
  foreign tag refused by name. `insert_log` gains `append_item`/
  `read_items` taking the tag explicitly; the record-facing `append`/
  `read` are unchanged wrappers.
- `LNK-FR-004` **Fold at open (`Symmetric`).** `read_portable_edges` =
  blob then log, a logged pair already present (either orientation)
  skipped; `open(inner, edges, path)` merges the log into the caller's
  edges the same way before its existing stale check, so the blob
  rewrite covers every logged edge, then clears the log; `open_portable`
  inherits both; `create` clears a stale log. A reopen with no links
  writes nothing.
- `LNK-FR-005` **`MultiSymmetric::link` and open labels.** Validates
  the label (`InvalidLabel`), then `LNK-FR-002`'s rules within that
  label's adjacency. A label with no adjacency yet is **created**: when
  the layer has a base path, an empty edge blob is written for it and
  the label manifest rewritten (temp-then-rename) *before* the edge is
  logged — so every manifest label has a blob and a crash between the
  two steps leaves an empty relation, not a dangling one; then the
  adjacency map for the label is created. `relation_kinds()` therefore
  reports the new label immediately.
- `LNK-FR-006` **The label manifest.** `<path>.relations`: a tagged
  blob (`GENLABL\0`, version 1, `R::SCHEMA_TAG`, fingerprinted body) of
  `Vec<String>` in first-seen order. Written by `create` (the initial
  labels) and on label creation; read by `open` and `open_portable`,
  whose effective label set is the caller's ∪ the manifest's, a
  manifest-only label starting from an empty caller list. A missing
  manifest is the empty set — every directory written before this round
  reopens exactly as before. A foreign or corrupt manifest is
  `RecordBlobUnreadable` naming its path.
- `LNK-FR-007` **Fold at open (`MultiSymmetric`).** Per label, exactly
  `LNK-FR-004`; `open_portable(inner, path, labels)` keeps its
  signature and adds the manifest's labels.
- `LNK-FR-008` **Every layer forwards.** `NameIndex`, `Reversed`,
  `MmapScanned` forward both traits to their inner store; `GenericProductionStore::link<R, Marker>` and `::link_by_relation<R>`
  under the write lock.
- `LNK-FR-009` **`ConnectionStore::link_records(&self, left, right,
  relation: &str) -> Result<LinkOutcome, ErrorCode>`** with a default of
  `Err(Unsupported)`; server-side `LinkOutcome::{Linked, AlreadyLinked}`.
  `Entity`: `valid_relation_label` else `Malformed`; `UnknownRecord` →
  `RecordNotFound`; `SelfLoop` → `Malformed`; `Durability` → `Storage`;
  any valid label is accepted (open labels). `Employee` (`research`):
  only `collaborates_with`, else `Malformed`; the `Symmetric` path.
- `LNK-FR-010` **`Request::Link { left, right, relation: String }`** at
  index 22, protocol 14. Answered `Response::Ok` for `Linked` *and*
  `AlreadyLinked` — insert-or-ignore, the consumer's own semantics
  (F10), so a retried link is safe; `Err` per `LNK-FR-009`. Gated as
  `Insert` is: `Malformed` below 14 (rule 3), `Unauthorized` for
  `ReadOnly`, `SessionOpen` inside a session, never journaled or staged.
  `audit::RequestKind::Link`. No new `ErrorCode`, no new response.
- `LNK-FR-011` **Discovery follows.** `Entity`'s `list_relation_kinds`
  already reads the layer's adjacency keys, so `ListRelationKinds`,
  `DescribeRelations`, `NeighborsByRelation`, `Join … ON <label>`, and
  `traverse` all see a runtime-created label with no further change.
- `LNK-FR-012` **Clients.** `SchemaDrivenClient::link(&mut self, left,
  right, relation: &str) -> Result<(), ClientError>`: `Unsupported("Link
  on this domain")` when the schema reports `relations.neighbors:
  false`; `Unsupported("link")` below 14 with no frame; after an `Ok`
  for a label absent from the cached relation list, the list is
  re-fetched (`DescribeRelations`) so a following `JOIN … ON <label>`
  compiles. Python `Client.link(left, right, relation)`,
  `PROTOCOL_VERSION = 14`.
- `LNK-FR-013` **Pins.** One golden vector (`Request/Link`),
  `PROTOCOL_VERSION` 13 → 14, table row 14, `SERVER-002` 0.3.0, the
  literal pins moved.

## Considered options

**Label model.** (a) **(proposed)** open labels with a manifest —
matches F3 and makes the label set `SELECT DISTINCT relation`, as
`ADR-0042` foresaw. (b) Fixed labels only (`RELATION_LABELS`) — cheaper,
but leaves the model `ADR-0042` identified as *the* mismatch in place
and forces the bridge to pre-declare every predicate. (c) Open labels
without a manifest, discovered by scanning the directory for
`<base>.*.edges` — fragile against unrelated files and the `.records`/
`.inserts` siblings; a manifest costs one small file.

**Where a new edge's content lives.** (a) **(proposed)** an edge log
beside each blob, folded at open — the insert-log design applied
again, no format change. (b) Rewrite the edge blob per link — O(edges)
per write. (c) Log only, no blob — a format change for the read path
`STORAGE-016` pinned.

**Duplicate edge.** (a) **(proposed)** `AlreadyLinked` at the layer,
`Ok` on the wire — the consumer's `INSERT OR IGNORE`. (b) A new
`ErrorCode` — makes a retry look like a failure.

**Label validation.** (a) **(proposed)** a filesystem-safe charset at
the boundary, `Malformed` otherwise. (b) Percent-encode labels into
file names — allows any label but changes the existing
`<path>.relates_to.edges` layout or needs two naming rules.

**Direction.** (a) **(proposed)** none; encode in the label. (b) A
directed relation kind — a new layer, a new `Neighbors` shape, its
own round.

## Proposed shape

```rust
// query.rs
pub trait Link<R: SymmetricRelation<Marker>, Marker> {
    fn link(&mut self, a: R::Id, b: R::Id) -> Result<LinkOutcome, LinkError<R::Id>>;
}
pub trait MultiLink<R: Record> {
    fn link(&mut self, relation: &str, a: R::Id, b: R::Id)
        -> Result<LinkOutcome, LinkError<R::Id>>;
}
// mod.rs
pub enum LinkOutcome { Linked, AlreadyLinked }
pub enum LinkError<Id> { UnknownRecord(Id), SelfLoop(Id), InvalidLabel(String), Durability(DurabilityError) }
```

`Symmetric` gains `edges_path: Option<PathBuf>` (set by `create`/
`open`/`open_portable`, `None` from `new`); `MultiSymmetric` gains
`base_path: Option<PathBuf>`. Neither field changes any existing
constructor's signature or behavior beyond the fold.

`Symmetric::link` (in the `SchemaTag`-bounded block):

```text
if inner.get(a).is_none() -> UnknownRecord(a); same for b; a == b -> SelfLoop
if adjacency[a] contains b -> AlreadyLinked
if let Some(path) = edges_path { insert_log::append_item(&log_path(path), R::SCHEMA_TAG, &(a, b))? }
adjacency[a].push(b); adjacency[b].push(a); Linked
```

`MultiSymmetric::link(label, a, b)`: `valid_relation_label` else
`InvalidLabel`; the record checks; if `adjacency` lacks `label`: with a
base path, write an empty `EdgeBlob` at `<path>.<label>.edges`, then the
manifest (labels ∪ {label}); insert an empty map; then the `Symmetric`
steps against that map with `<path>.<label>.edges.inserts` as the log.

`label_manifest` (`pub(crate)`, in `store.rs` beside
`labeled_edges_path`): `manifest_path`, `read(path, tag) -> Vec<String>`
(missing → empty), `write(path, tag, labels)` via `EncodedRecordBlob::
write`. `MultiSymmetric::open`/`open_portable` union the manifest in.

Server: `pub enum LinkOutcome` beside `InsertOutcome`;
`ConnectionStore::link_records` default; `dispatch`'s arm;
`handle_connection`'s three gates gain `Request::Link`; `RequestKind::
Link`. `EntityConnectionStore::link_records` → `store.link_by_relation::
<Entity>(relation, left, right)`; `EmployeeConnectionStore::link_records`
→ `store.link::<Employee, CollaboratesWith>(left, right)` for its one
label.

Client: `link` as `LNK-FR-012`; `refresh_relations` is private, called
only on that path. Python: `Client.link`, gated by
`REQUEST_INTRODUCED_AT[Link] = 14`.

## Data/state and invariants

- An edge is in a layer's adjacency (both directions) iff it is in the
  blob or the log; `link` writes the log before the adjacency, so a
  crash leaves at worst a logged edge the next open folds in.
- A label is in `MultiSymmetric::adjacency` iff its blob exists, and
  every label with a blob written by this crate is in the manifest;
  a label named by a caller at reopen that the manifest lacks is added
  to the manifest by `open` (the caller's list is authoritative as it
  always was, and now also durable).
- `AlreadyLinked` means nothing was written at any layer.
- No existing file changes format; a store nothing was linked in has
  no `.inserts` edge logs and — for `MultiSymmetric` — gains a
  manifest only when first opened by this version (written by `open`
  so the caller's labels are durable from then on), which every older
  reader ignores.

## Errors, failure, recovery, and observability

- `LinkError::UnknownRecord` / `RecordNotFound`; `SelfLoop` and
  `InvalidLabel` / `Malformed`; `Durability` / `Storage` — the insert
  round's mapping.
- A torn edge-log tail is dropped at read (never acknowledged); a
  foreign or corrupt log or manifest is `RecordBlobUnreadable` naming
  its path.
- Every `Link` reaches the access log as `RequestKind::Link`; a
  refused `ReadOnly` link reaches the audit log as `Refused`.

## Security, privacy, and compatibility

- The label is external input that becomes part of a file name; it is
  validated at the adapter before any layer sees it, and again at the
  layer (`InvalidLabel`) so no in-process caller can bypass it.
- `ReadOnly` cannot link; the gates are unchanged in kind.
- Wire: append-only, one version, `Malformed` below 14, the client
  never sends below 14. Every vector ≤ 13 is byte-identical.

## Acceptance criteria

1. `Symmetric` over a durable core: `link` is visible to `neighbors`
   in both directions immediately; a repeat is `AlreadyLinked` with the
   log unchanged; a self-loop and an unknown id are refused with
   nothing written; after reopen from the files alone the edge is
   present, the log gone, the blob rewritten; a second reopen writes
   nothing.
2. `MultiSymmetric` (`Entity` stack): a link under an existing label;
   a link under a **new** label creates it — `relation_kinds` lists it,
   its blob and the manifest exist; `open_entity_production_stack_
   portable` finds the new label with no caller change; an invalid
   label (`../x`, a space, empty, 65 bytes) is refused before any file
   is touched; a directory written before this round reopens unchanged.
3. Over a socket on `Entity`: `link` then `neighbors`/`neighbors_by_
   relation`/`ListRelationKinds`/`DescribeRelations`/`Join … ON <new
   label>`/`traverse` all see it; the repeat is `Ok`; unknown id
   `RecordNotFound`; self-loop and bad label `Malformed`; `ReadOnly`
   `Unauthorized`; in a session `SessionOpen`; a server restarted on the
   same directory serves the edge and the label. On `Employee`
   (`research`): a `collaborates_with` link over the wire, any other
   label `Malformed`. On `Dog`/`Reminder`: `Unsupported`.
4. `Hello { 13 }` and a silent client are answered `Malformed` with
   nothing linked; the Rust client refuses below 14 with no frame; the
   Python client links an edge the Rust client then sees.
5. Full sweep green, docs at the warning baseline.

## Verification plan

Unit: `store.rs` (`Symmetric` criterion 1 over `BaseStore`+blob, the
manifest, `valid_relation_label`, `MultiSymmetric` label creation and
fold), `entity.rs` (criterion 2), `employee_impl.rs` (`research`, the
`Symmetric` path through `Reversed`), `insert_log.rs` (tagged items),
`server/{entity,employee,serve}.rs`, `protocol.rs` (the vector, the
pin). Integration: `server_entity_integration`, `server_employee_
integration`, `server_protocol_version`, `server_dog_integration`,
`server_reminder_integration`, `server_auth_integration`, `server_
transaction_integration`, `server_python_client`.

## Traceability

- Roadmap: `SERVER-LINK-DESIGN`, `SERVER-LINK`. Decision: `ADR-0047`.
- Specification: `SERVER-001` v0.37.0 / `FR-047`; `SERVER-002` v0.3.0.
- Requirements: `LNK-FR-001`–`013`.

## Open questions

- Directed edges (F4) — when a consumer query needs subject/object
  and a label pair does not do; its own round.
- Whether a label should be normalized (lowercased) rather than only
  validated — the consumer keeps case and matches case-sensitively, so
  this round matches that; revisit if a bridge needs otherwise.
- Runtime compaction of edge logs — the record log's own revisit
  trigger, shared.

## Change history

- 2026-09-06: Accepted as designed (option (a)), PR #199. No content
  change.
- 2026-09-06: Implemented as `SERVER-001` v0.37.0 / FR-047, landed as
  designed.
- 2026-09-06: Initial proposal; implementation follows on the same branch. The
  twelfth round in the `rusty_remind_me`-motivated line; the one
  `ADR-0042` F3 named.
