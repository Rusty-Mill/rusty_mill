# ADR-0137: `Fair Play` domain — a card tree with explicit owners over a read-only baseline

- Status: **Proposed and implemented on one branch** (2026-10-04, the
  `ADR-0048`/`ADR-0049` cadence). The owner's prompt fixed most of the
  model; the decisions it left open are listed under "Decisions the
  prompt left open" and are what acceptance is for. No wire change.
- Date: 2026-10-04
- Deciders: baileyrd
- Related: `ADR-0048` (`Memory`: the one-index/one-scan stack, a
  `StrList` field, the read-only remainder), `ADR-0049` (`Replace<R>`:
  the main write path here), `ADR-0050` (`serve_tables`: how the three
  tables are served), `ADR-0051` (runtime deletion: what an orphan looks
  like), `ADR-0128` (nullable columns: the four optional ids on the
  wire), `ADR-0095` (the `SIGKILL` harness the split test copies),
  `crate::generic_spike::{rule_trace, source}` (the optional-parent
  `ChildOf` tree and `chain_to_root`).
- Supersedes/Superseded by: none. Adds one domain of three tables, one
  example, one benchmark example, two binaries; `Reversed::inner` on the
  engine; no wire change (protocol stays 36).

## Context

Eve Rodsky's *Fair Play* deals 100 household-task cards between two
partners. Two rules of the game drive the data model. **Whoever holds a
card owns all of its Conception, Planning and Execution (CPE)** — the
three are never assigned separately. **A card can be split**: "Cleaning"
becomes "Bathrooms" held by one partner and "Floors" by the other, each
a full card with its own CPE, minimum standard of care and owner. A
family also edits the shipped text, and wants to know what it changed
and to put it back.

The library has what this needs: an optional-parent `ChildOf` tree with
`Parent`/`Children`/`chain_to_root` (`Rule`, `Source`), a `StrList`
field and a bounded text-heavy record on the one-index/one-scan stack
(`Memory`), whole-record `Replace` (`ADR-0049`), more than one table on
one listener (`ADR-0050`). It lacks a second `Children` on one stack
(E0119) and any domain invariant beyond the id.

Where the precedents live decides placement: `Memory` and `Reminder`
are in the application crate's `src/generic/`, not the engine
(`ADR-0124` moved only the machinery). `Fair Play` goes to
`src/generic/fair_play.rs` beside them. **`serve_tables` has landed**
(`ADR-0050`, with `Use`/`ListTables`/cross-table `Join`), and so has
runtime deletion (`ADR-0051`), so the prompt's "open `ADR-0036`
clause" is closed; the deferred hooks below say what deletion still
cannot do for this domain.

## Decision

Three tables, in `src/generic/fair_play.rs`:

- **`Person`** `{ id, name, player }` — `name` indexed (the seed loader
  and a front end resolve by it), `player` scannable (the seat the
  deck's `(Player 1)`/`(Player 2)` cards refer to; 1-based file order).
- **`CardDefault`** `{ id, number, name, suit, conception, planning,
  execution, minimum_standard_of_care }` — one row per original deck
  card, the text as shipped. `suit` indexed, `number` scannable.
  Written by the seed loader, then **read-only by convention**: the
  module exposes no update or replace path for it, and the
  `card_default` wire adapter answers `Unsupported` to every write. The
  library has no enforced immutability: a caller holding the raw stack
  can `Replace<CardDefault>`. Stated, not implied.
- **`Card`** `{ id, number: Option<u16>, name, suit, parent_card_id:
  Option<Uuid>, position: u32, owner_id: Option<Uuid>, conception,
  planning, execution, minimum_standard_of_care: Vec<String>, notes,
  origin: Deck | Family, baseline_id: Option<Uuid> }` — `suit` indexed,
  `position` scannable, three `ChildOf` relations: `ParentCard`
  (self-referential, the tree), `OwnedBy` (into `Person`), `BaselineOf`
  (into `CardDefault`).

**CPE is three fields, not a child table.** Nothing ever assigns
Conception apart from Execution; a child table would let the data say
something the game forbids.

**`Card` is a self-referential tree; there is no `Task` table.** A
split-off card is the same record type as the deck card it came from.
Every query — held by, by suit, balance, state — is then one query over
one table at any depth, and a front end draws one kind of card.

**Origin and baseline; customization is derived, never stored.** Every
deck card points at its `CardDefault`. `card_state` computes
`Original` (deck card, six text fields equal the baseline), `Edited`
(deck card, any of the six differs) or `Custom` (`origin == Family`) by
comparison, never from a flag that could drift. The six are `name`,
`suit`, `conception`, `planning`, `execution`,
`minimum_standard_of_care`; `notes`, `owner_id`, `parent_card_id` and
`position` are play state or annotation and never make a card `Edited`.
Splitting leaves the parent `Original`; its children are `Custom`.
`reset_to_baseline` puts the six back through one `Replace`, keeping
owner, parent, position and notes. Invariant: `origin == Deck` iff
`number.is_some()` iff `baseline_id.is_some()`.

**Ownership is explicit on every card and never inherited.** At most
one owner per card, holding all CPE for that card. A parent may have a
different owner from its children, or none: the parent's owner holds
what is left at that level. The real "still undealt" list is the
**unowned leaf cards**; `balance` has a leaf-only variant because a
split parent and its children would otherwise count the same work
twice. CPE text is not copied from parent to child on a split.

**Validated writes.** `insert_card`/`replace_card` check the invariant
above, that the card is not its own parent, that the parent exists,
that the new parent chain does not reach the card (no cycles), and on
replace that `number`/`origin`/`baseline_id` are unchanged. Every
domain operation — `reassign_card`, `split_card`, `create_custom_card`,
`reset_to_baseline` — goes through them. The raw `Insert`/`Replace`
traits bypass them: the stack cannot forbid a cycle, a flipped origin
or a duplicate `number` (`origin_number_baseline_invariant_is_rejected_
on_domain_writes_only` writes both and shows them). On the wire the
`card` adapter routes `Insert`/`Replace` through the same two
functions, so a client cannot bypass them.

**The stack.** `Reversed<Reversed<GenericMmapStore<Card, SuitField,
PositionField>, Person, Card, OwnedBy>, Card, Card, ParentCard>`. The
outer `Reversed` answers `Children` for `ParentCard` only; the inner
one's `Children<Person, Card, OwnedBy>` is reached by a concrete
forwarding impl on the stack type through a new `Reversed::inner`
accessor (the `Ordered::inner` precedent) — the per-pair technique
`forward_scannable_pairs!` exists for, written by hand since there is
one pair. `BaselineOf` has no reverse index: no required query walks
from a baseline to its card, and `Parent` is free.

**`split_card` is not atomic, and is ordered so it never has to be.**
Children are inserted first, one at a time, each already pointing at
the existing parent; the parent is replaced last, and only if the
caller asked. A crash after `k` children leaves `k` children under the
unchanged parent: valid. A crash during the parent's replace leaves the
children and `ADR-0049`'s one-field window (the log entry lands before
the slot write, so at worst `position` is the old value). Nothing ever
dangles. A rerun with the same child ids is `Duplicate` on the first
one present, so a caller resumes by skipping the children already there
(`an_interrupted_split_is_resumed_by_skipping_the_children_already_
present`). `split_card_with` reports each durable step to an observer;
that is what the `SIGKILL` test pauses on.

**Deterministic ids.** `fair_play_id(kind, key)` is SHA-256 over
`fair_play:<kind>:<key>` (the `entity_id` recipe, `ADR-0042`, with a
namespace): `person_id(name)`, `card_default_id(n)`, `deck_card_id(n)`,
`split_card_id("17/Bathrooms")`. This is what makes the seed loader
idempotent by `number` without a second index, and makes "card by deck
number" one `GetById` when the convention holds.

**Seed loader** (`examples/fair_play_seed.rs`, logic in
`examples/support/fair_play_seed_lib.rs`, the `restore_backup`
placement): `data/fair-play-cards.csv` is the supplied file, copied in
and loaded verbatim; `data/fair-play-people.csv` and
`data/fair-play-splits.csv` are header-only until supplied. Hand-rolled
RFC 4180 CSV rather than a dependency. Fails loudly on a row count
other than 100, suit counts other than 22/22/22/22/10/2, a repeated or
out-of-range `number`, an unknown suit, an empty name, a wrong header —
each by file and 1-based line. Every input is parsed and checked before
the first write. For each deck row it creates one `CardDefault` and one
live `Card` (`origin: Deck`, `baseline_id` set, text copied, the row's
`notes` on the live card only). A rerun creates nothing already there
and **never overwrites a live card** — a family's edits, owners and
splits survive — nor a baseline, even if the file's text changed. A
split row resolves `parent_path` (a deck number, then child names),
`owner_name` by the `Person` name index, standards split on `|`, and
positions by file order through `split_card`.

**Server** (`src/server/fair_play.rs`, `src/bin/fair_play_server.rs`):
three adapters registered through `serve_tables` as `card` (primary),
`person`, `card_default`. Ids cross the wire as `Str`; `number`,
`parent_card_id`, `owner_id` and `baseline_id` are nullable
(`ADR-0128`) over the sentinels `0`/`""`. The `card` table's one wire
relation is the tree (`parent`/`children`); `owner_id` and
`baseline_id` are plain filterable fields, because the wire's single
`Parent`/`Children` pair carries one relation per table. See "Gaps".

## Decisions the prompt left open

- **The scannable field is `position`, as `u32`.** It is the one field
  that changes alone (a sibling reorder), so `UpdateField` moves it in
  place; `number` never changes and `origin` has two values, so neither
  earns the slot. `u32` rather than the prompt's `u16` because the slot
  file holds `u32`/`i64`/`Uuid` only (`MmapFieldValue`) and a
  narrowing `set_scannable_value` would have to panic or truncate.
  `CardDefault::number` stays `u16` and is scanned as `u32`.
- **`Person` gets a `player` field.** Every table on this stack needs
  one scannable field; the seat number is real (the deck says
  `(Player 1)`), is derived from file order, and invents no data.
- **No `root_card_id`.** Measured (`examples/fair_play_bench.rs`,
  release, one run on this machine, 227 cards):

  | depth | chain walk (`root_card_id`) | one `GetById` (what a stored root costs) | ratio |
  |---|---|---|---|
  | 1 | 442 ns | 162 ns | 2.7× |
  | 2 | 614 ns | 162 ns | 3.8× |
  | 3 | 891 ns | 163 ns | 5.5× |
  | 4 | 1.11 µs | 162 ns | 6.9× |
  | 5 | 1.28 µs | 161 ns | 7.9× |

  The ratio grows with depth as the earlier parent-chain benchmarks
  showed, and never matters: a depth-5 walk costs 1.3 µs, `card_tree`
  of an 11-card tree 3.8 µs. A denormalized root would add an invariant
  ("every descendant's root equals its ancestor's") that a re-parent
  must maintain across the whole subtree, with no transaction to do it
  in. Not worth a microsecond. Revisit if a tree ever has thousands of
  cards or a depth past ten.
- **State queries are a scan, and it does not matter.** `suit` is the
  one index. An `origin` index would only split `Custom` from the rest;
  `Original` vs `Edited` needs every card's baseline either way.
  Measured (same run; one card in ten `Edited`):

  | cards | `cards_by_state(Edited)` | `state_counts` |
  |---|---|---|
  | 100 | 103 µs | 107 µs |
  | 500 | 609 µs | 646 µs |
  | 2 000 | 4.1 ms | 3.3 ms |
  | 5 000 | 18 ms | 17 ms |

  About a microsecond a card: one boxed-record clone and one baseline
  clone each. A household has a hundred-odd cards and a few hundred
  after splits, so a state view costs well under a millisecond. No
  second index; the stack supports one and the gap is not worth
  closing for this domain.
- **The domain exposes no delete.** `Delete` exists (`ADR-0051`) and
  `Reversed` forwards it; the domain adds no `delete_card` because the
  only sensible deletes (merge/unsplit, retiring a person who holds
  cards) need decisions it has no transaction for. See "Gaps" for what
  a raw delete leaves.
- **Card ids are the caller's.** `SplitSpec`/`NewCustomCard` carry the
  id; the loader mints them deterministically. `Uuid::new_v4` is not
  enabled in this crate (reproducible datasets), and a server-side
  mint would be a wire change.

## Gaps, named

- **The stack cannot enforce** uniqueness of `number`, acyclicity of
  the parent chain, or the origin/number/baseline invariant; the domain
  functions do, the raw traits do not. Tested in both directions.
- **An orphan after a raw delete** (`ADR-0051`: records do not cascade):
  the children keep their `parent_card_id`, `Parent` names an id with
  no record, `chain_to_root` stops at the cold trail, and the children
  index still lists them under the deleted id — live and after a
  reopen, since it is rebuilt from the children's fields. A later
  domain write of an orphan is refused as `ParentNotFound` until it is
  re-parented. (`ADR-0051`'s comment that a deleted parent "loses its
  children list" describes the design, not the code; the index entry
  stays. Recorded here, not changed.)
- **The wire carries one relation per table.** `owner_id` and
  `baseline_id` are fields; "cards held by a person" over the wire is a
  `Query` with a predicate on `owner_id` (a scan), not the `OwnedBy`
  index the embedded API uses. A front end on the socket pays a scan of
  a few hundred rows; one on the embedded API does not. A second wire
  relation per table is a protocol question for its own round.
- **`card_default` read-only-ness** is a wire rule and an in-process
  convention, not a type.
- **Owner and baseline existence** are not checked by `insert_card`/
  `replace_card`: they live in other stores. The loader checks the
  owner; the server checks nothing across tables for these two fields.
- **A `number` uniqueness** violation written through the raw trait
  shows in `cards_by_number`, which returns every match for that reason.

## Deferred hooks (named, not built)

- **Merge/unsplit.** Collapse children back into their parent: delete
  the children (`Delete`, available) after deciding who holds the
  merged card and what happens to each child's edited text. Two or more
  writes with no transaction; order them children-first as `split_card`
  does, and decide the owner before the first delete.
- **Deal history.** An append-only `Assignment { id, card_id,
  person_id, dealt_at }` table layered beside `Card`: `reassign_card`
  would insert one row then `Replace` the card, the same two-step shape
  as a split, with the card's `owner_id` as the current truth and the
  table as the log.
- **Per-card status or recurrence.** Not Fair Play's job: the deck
  divides ownership; it is not a checklist.
- **Default CPE from parent** on a split: a copy at creation would be
  one line in `split_card`; it was left out so a split-off card's text
  is visibly the family's own.
- **A second wire relation** (`owner`) for the `card` table.

## Consequences

- Positive: the whole model — tree, owners, baseline, state — is
  expressed with the traits the library already has; one engine
  accessor (`Reversed::inner`) and no new dependency, file format or
  wire variant.
- Positive: every required query (1–16) is a function over the generic
  traits, usable against the raw stack, the `GenericProductionStore`
  wrapper, or an in-memory composition.
- Named, not hidden: `split_card` is two or more durable steps; the
  windows are listed above and tested with a real `SIGKILL`.
- Named, not hidden: state views scan; measured, and cheap at this
  domain's size.
- Named, not hidden: owners are reachable by index in-process and by
  scan over the wire.

## Considered options

**(a) Accept as designed** — the tree, explicit owners, derived state
over a read-only baseline, `position` scannable, no `root_card_id`, no
delete. **(b) A `Task` table under `Card`** — two record types for one
kind of thing; every query twice. **(c) A stored `customized` flag** —
drifts from the data on the first edit that misses it. **(d) A
denormalized `root_card_id`** — measured, not worth its invariant. **(e)
CPE as three child rows** — lets the data say what the game forbids.
**(f) Decline.**

## Acceptance and implementation

- 2026-10-04: proposed and implemented on one branch —
  `src/generic/fair_play.rs` (the domain, queries 1–16, `split_card`,
  `reset_to_baseline`, state derivation), `src/generic/mod.rs`,
  `rusty_multimodal_db_engine/src/generic/store.rs` (`Reversed::inner`),
  `examples/fair_play_seed.rs` + `examples/support/fair_play_seed_lib.rs`,
  `examples/fair_play_bench.rs`, `data/fair-play-{cards,people,splits}.csv`,
  `src/bin/fair_play_crash_writer.rs`, `tests/fair_play_crash.rs`,
  `tests/fair_play_seed.rs`, `src/server/fair_play.rs`,
  `src/bin/fair_play_server.rs`, `tests/server_fair_play_integration.rs`,
  `clients/python/fair_play_driver.py`, `Cargo.toml`. Tests: eleven in
  the domain (every required query; persist/drop/reopen; reassign then
  reopen; a four-level tree with a different owner at each level and
  the leaf-only balance; state derivation for all six fields and the
  change-it-back case; reset; the invariant and the raw-trait gap;
  cycles; orphans), four for the loader (the supplied deck verbatim,
  rerun idempotency with an edit surviving, splits by path, refusals by
  line), three for the crash harness (control, kill after each step ×3
  trials, resume), the adapter's unit tests, and the socket suite with
  the Python driver. `SERVER-001` v0.110.0 / `FR-123`, `FPL-FR-001`–`008`.
- 2026-10-04: the merge/unsplit hook is built, in the domain crate:
  `unsplit_card` deletes the subtree deepest first and keeps the parent
  (the owner stays the parent's; a child's edited text goes with it),
  beside `delete_card` (leaves only), `delete_person` (holding nothing)
  and `reorder_children` (one call, exact set). "The domain exposes no
  delete" above now reads: no unguarded delete.
- 2026-10-04: the front end, `crates/apps/rusty_fair_play` — a JSON HTTP
  API over the embedded stacks on `rusty_http` (rusty_tick's sans-IO
  router; the TCP adapter is now the shared `rusty_serve`) and a React web UI in its `web/`. To let an
  app crate depend on the domain under ADR-0003's layer rule (no app
  depends on another family's app crate), the domain moved out of this
  crate into `crates/libs/storage/rusty_fair_play_domain`, re-exported
  here as `generic::fair_play` exactly as the engine is (ADR-0124); the
  seed loader moved from `examples/support/` into it as `seed` with the
  deck embedded as `DECK_CSV`, and the seed CLI, the benchmark example,
  the crash writer and the domain's tests went with it. The wire
  adapters, `fair_play_server` and the socket suite stay here. Its own
  decision record is `crates/apps/rusty_fair_play/docs/decisions/ADR-0001-front-end-shape.md`.
