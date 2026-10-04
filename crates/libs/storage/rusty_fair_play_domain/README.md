# rusty_fair_play_domain

Eve Rodsky's **Fair Play** household-card system as a domain on
[`rusty_multimodal_db_engine`](../rusty_multimodal_db_engine): three
records — `Person`, `Card` (a self-referential tree with an explicit owner
per card) and `CardDefault` (the shipped text a family's edits are measured
against) — their durable stacks, every query a card-style front end needs,
and the seed loader with the 100-card deck embedded. Designed and measured
in `rusty_multimodal_db`'s
[ADR-0137](../../../apps/rusty_multimodal_db/docs/decisions/ADR-0137-fair-play-domain.md);
a libs crate so that both `rusty_multimodal_db` (which re-exports it as
`generic::fair_play` for its wire adapters) and the
[`rusty_fair_play`](../../../apps/rusty_fair_play) web app can depend on it.

Rules: whoever holds a card owns all of its Conception, Planning and
Execution (three fields, not a child table); a card can be split into cards
with different owners; the real "still undealt" list is the unowned leaf
cards; a deck card is `Original`, `Edited` (any of six text fields differs
from its baseline) or `Custom` (family-made), computed, never stored.
`insert_card`/`replace_card` refuse the origin/number/baseline invariant,
self-parents, missing parents and cycles; `split_card` inserts children
first and replaces the parent last, so every crash prefix is a valid store.
Deletes are guarded so nothing is orphaned: `delete_card` takes a leaf,
`unsplit_card` a whole subtree deepest first (the parent stays),
`delete_person` someone holding nothing; `reorder_children` checks an exact
list of the children, then writes each position in turn (not crash-atomic).

```sh
cargo test -p rusty_fair_play_domain                       # unit, seed and SIGKILL-split tests
cargo run -p rusty_fair_play_domain --example fair_play_seed -- <dir> [--people p.csv] [--splits s.csv]
cargo run --release -p rusty_fair_play_domain --example fair_play_bench   # the ADR's measurements
```

`data/fair-play-cards.csv` is the supplied deck, loaded verbatim and
embedded as `seed::DECK_CSV`; `fair-play-people.csv` and
`fair-play-splits.csv` are header-only until supplied.
