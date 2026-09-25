# rusty_multimodal_db_engine

The embedded, mmap-backed generic record store from
[`rusty_multimodal_db`](../../../apps/rusty_multimodal_db), extracted into
the libs layer so another product can embed it without depending on that
app crate (its [ADR-0124](../../../apps/rusty_multimodal_db/docs/decisions/ADR-0124-engine-extracted-to-libs.md)).
The first consumer outside `rusty_multimodal_db` is `rusty_remind_me`'s hub
([its ADR-0021](../../../apps/rusty_remind_me/docs/adr/0021-hub-storage-moves-to-rusty-multimodal-db.md)).

## What's here

- `generic::traits`: what a record type implements (`Record`,
  `IndexedField`, `ScannableField`, `OrderedField`, `SchemaTag`, …).
- `generic::store`: composable layers (`Indexed`, `Scanned`, `Ordered`,
  `MultiSymmetric`, `NameIndex`, …) that stack over a core store.
- `generic::GenericMmapStore`: the durable core. It keeps records in memory,
  one scannable field in an mmap'd slot file, and the full records in a
  tagged, fingerprinted companion blob, with an fsync'd insert log for
  writes between opens.
- `generic::GenericProductionStore`: a composed stack behind one `RwLock`,
  shareable across threads.
- `durability::DurabilityError`, and the blob header helpers every
  companion blob shares.
- `codec`: the one bincode configuration every on-disk format here uses.

The full design, requirements and ADRs are in `rusty_multimodal_db`'s
`docs/`, where this code was developed. `rusty_multimodal_db` re-exports all
of it under its original paths.

## Constraints worth knowing

- **Everything lives in RAM.** Each store keeps every record in a `HashMap`.
- **Ids are `u32`, `i64` or `Uuid`** (`MmapFieldValue`), and ids and
  ordered-index keys must be `Copy`.
- **One writer at a time.** `GenericProductionStore` holds a single
  `RwLock` and panics if a previous holder panicked. A caller that must
  survive a panic should own its lock.
- **The insert log grows until the next `open` or `compact()`,** so a
  long-running process should compact on a schedule.

## Testing

```sh
cargo test -p rusty_multimodal_db_engine                     # without the Order fixture
cargo test -p rusty_multimodal_db_engine --features research # the full suite
```
