# `rusty_crdt`: nexus-crdt reading, ADR check and API sketch

Steps 1 and 2 of the `rusty_crdt` extraction. Starts from `docs/bbp-concurrency-design.md` (§f,
Appendix A, Appendix B) on `main` at `d30f3681`.

> **Decision (owner, 2026-10-26):** build, not park. The crate follows §3 with
> the review fixes and lands in #1122. §4's recommendation to park is kept as
> the record of the alternative. Re-pointing `rusty_bbp` is a separate change,
> and the Nexus adapter stays blocked while `FROZEN.md` stands.

## 0. What changed since the 2026-10-13 design

Read this first: it decides the recommendation in §4.

- **The branch is gone.** `claude/bbp-concurrency-design` no longer exists on
  the remote; the design note merged to `main` through #834. This note builds
  on `main`.
- **The 2b pilot verdict was withdrawn.** That pilot was poll-driven; BBP is now
  event-based (`docs/bbp-event-based-plan.md`, steps 1 to 3c merged).
- **A private RGA already ships in `rusty_bbp`.** Step 4a (#1052) added
  `crates/libs/protocol/rusty_bbp/src/coedit.rs`: line ops with a stable
  `LineId { art, n }`, tombstones, and a fold in log order. A proptest
  (`tests/coedit.rs:337`, `the_log_order_fold_is_what_rga_converges_to`) checks
  that the fold equals a textbook array RGA fed the same ops in any causally
  valid order. The test oracle is a second, independent RGA.
- **Contention is low.** The line-level check (#1067) refuses 0.03 edits per
  accepted edit with 4 editors (3% wasted) in the scripted load harness.
- **ADR number.** Appendix B's provisional "ADR-0010" is now taken
  (`docs/adr/0010-keep-dev-only-and-proc-macro-crates.md`); the next free number
  is 0012.
- Mill Board cards #20 and #23 (`rusty_crdt`, CRDT co-editing) sit in Backlog.

## 1. Reading note: `nexus-crdt`

All 12 source files read in full (3,623 lines, tests included), not only the
module list. `nexus-crdt` is an op-based CRDT for the Nexus block editor. It has
three layers: a character RGA, generic causal-delivery machinery, and an editor
integration that sits on top of both.

| File | Lines | What it does | Kind | External deps used |
| --- | ---: | --- | --- | --- |
| `text.rs` | 486 | `RgaText`: a tree RGA of `char`s (algorithm below) | **RGA** | `serde` derives only |
| `id.rs` | 159 | `SiteId(Uuid)`, `Lamport(u64)`, `OpId { lamport, site }` (derived order: lamport first), `VersionVector` | generic CRDT infra | `uuid`, `serde` |
| `log.rs` | 346 | `OpLog`: idempotent append by `OpId`, VV summary, prune floor, union `merge`, `missing_for` gossip | generic CRDT infra | `serde` |
| `merge.rs` | 137 | `baseline_op_id` (UUID v5 ids for pre-existing text, lamport 0), `subop_id` (per-char ids in a multi-char insert), `byte_to_char_pos` | app | `uuid` |
| `doc.rs` | 1,087 | `CrdtDoc` over `nexus_editor::BlockTree`: one RGA per block, local and remote apply, concurrency test by VV, silent text merge, structural conflict detection | app | `serde`, editor types |
| `op.rs` | 139 | `CrdtOp`: editor `Operation` plus `OpId`, VV at creation, RGA translation | app | `serde`, editor types |
| `conflict.rs` | 147 | Delete-versus-edit and whole-block conflict descriptors | app | `serde` |
| `wire.rs` | 296 | Kernel event-bus topics and JSON envelopes | app | `serde_json` |
| `sync.rs` | 330 | `SyncLoop` pumping bus ops into a `CrdtDoc` behind a mutex | app | `tokio`, `tracing`, kernel |
| `state.rs` | 389 | On-disk persistence under `.forge/.editor/crdt/`, schema version, its own SHA-256 | app | `serde` |
| `error.rs` | 44 | `CrdtError` | app | `thiserror`, editor error |
| `lib.rs` | 63 | Re-exports | | |

**Where the dependencies are.** `tokio` is used only by `sync.rs`, `tracing`
only by `sync.rs`, `thiserror` only by `error.rs`, and `uuid` only by `id.rs`
and `merge.rs`. `serde` is mostly derives; JSON lives in `wire.rs`, `state.rs`
and `sync.rs`.

### The algorithm (`text.rs`)

- Each element is a node keyed by `OpId`, with an optional parent: the element
  to its left when it was authored, or `None` for the head.
- Siblings (nodes that share a parent) are kept sorted by **descending** id.
  Rendering is a pre-order walk: a node, then its children in order, so the
  newest sibling sits nearest its parent.
- Delete sets a tombstone flag. The node and its character stay, so later
  inserts anchored on it still resolve.
- `apply` is idempotent by id. Rendering and position lookup use an explicit
  stack, because a linear chain is as deep as it is long (regression test at
  300,000 elements).
- Local edits are positional: `build_insert(pos)` takes the id at visible index
  `pos - 1` as the parent, and `build_delete(pos)` targets the id at `pos`. Both
  are O(n) walks. `len()` is also O(n).

### Findings that matter for any port

1. **Unknown references are silently dropped.** An insert with an unknown parent
   or a delete with an unknown target returns `false` (`text.rs:236-243`,
   `:271`). `CrdtDoc` compensates with a pre-check
   (`doc.rs` `missing_rga_parent`) that was added after a divergence bug (test
   `remote_delete_before_target_insert_does_not_diverge`). A port must report
   this case and not drop the op.
2. **Intention needs fresh ids; convergence does not.** The tree shape depends
   only on the set of ops, so replicas converge for any unique, totally ordered
   ids. But a local insert lands directly after its parent only if its id is
   greater than every existing child of that parent. Nexus gets this from
   Lamport clocks (`apply_local` bumps, `apply_remote` takes the max).
   **Appendix A's "no trait, any `Clone + Ord` works" is wrong on this point:**
   a caller that reuses a small id gets an insert rendered in the wrong place,
   and law 6 (single-site model) fails. The sketch in §3 checks freshness.
3. **One id with two payloads makes replicas diverge.** The first op with an id
   wins, wherever it arrives first. Nothing detects it.
4. **`expect` in library code** (`text.rs:258`) is guarded by an earlier check,
   but it is still a panic site.
5. **Tombstones keep the value**, which the port keeps too (§3), and `len()` is O(n).
6. **`VersionVector::contains` means "max lamport seen"** (`id.rs:106`). It is
   sound only with in-order delivery per site, and so is the prune floor in
   `log.rs`.
7. `subop_id` gives characters 2..n of a multi-char insert the envelope's
   lamport with a hashed site id. They are fresh at authoring time (each is the
   only child of the character before it), so finding 2 holds.

### What ports cleanly without `uuid`, `serde` or `tokio`

- **Ports as is:** the whole `text.rs` algorithm, made generic over the id and
  element types. Its only external use is `serde` derives.
- **Shape ports, code stays:** the `(lamport, site)` id order. Each caller
  brings its own id type: Nexus its `OpId`, BBP its `LineId`.
- **Stays in Nexus:** everything else. `OpLog`, `VersionVector`, the prune floor,
  baseline and sub-op ids, `CrdtDoc`, conflicts, wire, sync and persistence each
  have one caller, and most are tied to the editor or the event bus.

## 2. ADR-0002 check

**Tier S allows a first-party workspace crate, with no amendment.** ADR-0002
defines "external" as a dependency resolved from crates.io or a git remote. It
also says "a crate's tier is a claim about its own manifest". `rusty_bbp`
already depends on four first-party crates this way: `rusty_serde`, `rusty_rsa`,
`rusty_atomic_file` and `rusty_diff`. The conditions are:

- `rusty_crdt` is itself Tier S, so the claim holds transitively.
- It resolves by path or `{ workspace = true }` against a path entry
  (`ATLAS-RWC-0050`, checked by the `dependency-policy` job).

**Optional serde.** An optional *external* `serde` feature would still appear
in `[dependencies]`, which breaks Tier S by the letter (option C in Appendix B).
The decided option A uses the workspace's own `rusty_serde` behind a feature,
following `rusty_uuid`. That keeps Tier S without amending the ADR. `rusty_bbp`
does not enable the feature: it serializes `LineOp` through its own event types.

**Vendoring** (option D) is what exists today: `coedit.rs`.

## 3. API sketch (revised Appendix A, unbuilt)

Changes from Appendix A:

- Fallible calls return `Result`.
- Freshness is checked (finding 2).
- Reusing an id with a different payload is an error, not silent divergence
  (finding 3). Tombstones therefore keep their value, and `T: Eq`.
- No snapshot-hash hook in the crate. BBP binds a verdict to a version
  `ArtId`, and a version's text is a deterministic fold of the log prefix
  (`the_plan_gate_binds_one_co_edited_version`). That makes the id the snapshot.
  A content hash, if one is ever wanted, is SHA-256 over `iter()`, computed in
  `rusty_bbp` with `rusty_rsa`, so the crate needs no hashing.

```rust
//! Sequence CRDT (RGA). Sync, no I/O, no dependencies.
//! Caller contract: ids are unique across replicas and totally ordered by `Ord`.
//! Among siblings the greater id renders first.

/// No custom trait: `Ord` is the sibling tiebreak and `Clone` is all the
/// arena needs. Nexus passes its `OpId` (already lamport-first), BBP its `LineId`.
pub struct Rga<I, T> { /* arena, BTreeMap<I, usize>, roots, visible count */ }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op<I, T> {
    /// Insert `value` after `parent`; `None` is the head.
    Insert { id: I, parent: Option<I>, value: T },
    /// Tombstone `target`.
    Delete { target: I },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied<I> {
    Changed,
    Duplicate,
    /// Parent or target unknown. State unchanged; the caller buffers or refuses.
    Pending { missing: I },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error<I> {
    /// `insert_at`/`delete_at` past the visible length.
    OutOfRange { pos: usize, len: usize },
    /// A local insert id not greater than every existing child of its parent:
    /// it would render in the wrong place (intention), though replicas would
    /// still converge. Checking the parent's children, not a global maximum,
    /// keeps Nexus's hashed sub-op ids valid (finding 7).
    NotFresh { id: I },
    /// The id is already held with a different parent or value.
    IdReused { id: I },
}

impl<I: Clone + Ord, T: Clone + Eq> Rga<I, T> {
    pub fn new() -> Self;
    pub fn len(&self) -> usize;                 // visible, O(1)
    pub fn is_empty(&self) -> bool;
    pub fn contains(&self, id: &I) -> bool;     // live or tombstoned
    pub fn iter(&self) -> Iter<'_, I, T>;       // visible (&I, &T) in RGA order, iterative
    pub fn id_at(&self, pos: usize) -> Option<&I>;                          // O(n)
    pub fn insert_at(&self, pos: usize, id: I, value: T) -> Result<Op<I, T>, Error<I>>;
    pub fn delete_at(&self, pos: usize) -> Result<Op<I, T>, Error<I>>;
    /// Idempotent; never panics. A tombstone keeps its value, so a replayed
    /// insert is still told apart from a reused id after a delete.
    pub fn apply(&mut self, op: &Op<I, T>) -> Result<Applied<I>, Error<I>>;
}
```

Appendix A also had an `id` on `Delete` that the RGA ignores. It is dropped:
the caller's envelope carries the op's identity. A multi-element insert is a
chain of `Insert`s, each parented on the one before.

**Manifest.** `crates/foundation/rusty_crdt`, no `[dependencies]`, a
`rusty_serde` feature off by default, `proptest` as a dev-dependency only,
`unsafe_code = "forbid"`, and `unwrap_used`/`expect_used` denied outside tests.
New workspace members are picked up by `affected_crates.py` for scoped CI; that
gets verified in step 3, not assumed.

**Payloads.** `T: Eq`, not `PartialEq`: replay idempotence needs equality to be
reflexive, and a `PartialEq`-only payload such as an `f64` NaN would make an
exact replay look like `IdReused`. Tombstones keep their value (Appendix A
dropped it), because `Duplicate` versus `IdReused` must still be decidable after
a delete: after `Insert(1, None, 'a')` and `Delete(1)`, replaying
`Insert(1, None, 'a')` is `Duplicate` and `Insert(1, None, 'b')` is `IdReused`.
The id and parent alone cannot tell them apart. The memory cost is what
`nexus-crdt` already pays.

**Law tests.** Appendix A's nine laws stand, with two changes. Law 9 is
tightened: a reused id with a different payload returns `IdReused`; it is no
longer just documented. A new law 10 covers replay after a delete: an identical
insert returns `Duplicate`, and a different payload or parent returns
`IdReused`, both leaving state unchanged.

**Oracles.** The generic laws need a parent-aware oracle: a plain tree walk,
or the single-site `Vec` model for law 6. `tests/coedit.rs`'s array RGA is
**not** a general oracle. It scans siblings by id and assumes ids increase
along causality, which holds for BBP's log-assigned `LineId`s but not for an
arbitrary `Ord` id. For example, take `Insert(10, root)`, `Insert(1, child of
10)` and `Insert(5, root)`. The tree renders `10, 1, 5`, while the array scan
gives `10, 5, 1`. That oracle may be used only on histories generated under
BBP's constraint.

## 4. Does extraction still have a forcing function?

**No. I recommend parking `rusty_crdt` rather than building it.** The
reasoning:

- **BBP needs no order-independent `apply`.** The task log is a total order.
  `LineId`s are assigned from the log, so they are always fresh, and the fold is
  already proven equal to RGA. A commutative `apply` buys delivery in any order
  and replicas without a sequencer, and BBP has neither.
- **What a CRDT would change is admission, and that needs no new crate.** Today
  an edit that shares an anchor with a newer edit is refused
  (`coedit::overlap`). Accepting it means relaxing that check, because the fold
  already places concurrent inserts and deletes deterministically in RGA order.
  The measured need is 0.03 refusals per edit.
- **Nexus is frozen.** `FROZEN.md` forbids new adapters, so the second call site
  does not exist.
- **So the live call sites that need the library number zero, not one.** The
  "fallback" is already in place: `coedit::Doc` has RGA semantics, but not the
  Appendix A API, and it does not need that API.

**Triggers to revisit**, recorded on cards #20 and #23:

- (a) A BBP store replicated without a single sequencer (several hosts, or
  offline editors), where ops arrive out of order.
- (b) Nexus is unfrozen and wants a shared core.
- (c) A second live consumer of sequence merge appears.

If you still want the crate for isolation alone, §3 is the API to build. It
fixes two defects (findings 2 and 3) that a straight port of `nexus-crdt` or of
Appendix A would carry.

## 5. Other findings (not acted on)

- `nexus-crdt/src/state.rs` carries its own SHA-256 even though `rusty_sha2`
  exists. Nexus is frozen, so no action.
- `coedit::Doc::apply` clones the whole doc per accepted version, and each
  version keeps an `Arc<Doc>`. Memory is O(versions × lines). This has not been
  measured on real specs.
- `docs/bbp-concurrency-design.md` §f lists "Private RGA inside `rusty_bbp`" as
  superseded, and Appendix A has BBP passing `(lamport, principal index)` ids.
  Both are stale after #1052; a pointer to this note is added at the top of that
  file.
- `rusty_multimodal_db` was not touched or read.
