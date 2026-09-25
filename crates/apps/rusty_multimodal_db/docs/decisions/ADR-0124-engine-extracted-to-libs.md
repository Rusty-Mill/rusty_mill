# ADR-0124: The Generic Store Is Extracted to a Libs Crate

- Status: **Accepted by the owner before implementation** (2026-09-24, as
  phase 1 of `rusty_remind_me`'s ADR-0021: "Embed, hub-owned types" and
  "Split out a libs crate"). No wire change, no on-disk format change.
- Date: 2026-09-25
- Deciders: baileyrd
- Related: `rusty_remind_me` `docs/adr/0021-hub-storage-moves-to-rusty-multimodal-db.md`
  (the consumer and its decisions), `ADR-0043` (declined a split *client*
  crate), `ADR-0062` ("Done" means a backend for `rusty_remind_me`),
  `ADR-0016`/`ADR-0019` (the record blob and its schema tag, whose shared
  header now lives in the engine).
- Supersedes/Superseded by: none. `ADR-0043` declined splitting out the
  network client; this extracts the embedded store, a different part,
  for a reason `ADR-0043` did not have: the workspace's layer check.

## Context

`rusty_remind_me`'s hub is moving its storage onto this crate's generic
store, embedded in the hub process with record types the hub owns (its
ADR-0021). The monorepo's layer check
(`.github/scripts/check_workspace_layers.py`) forbids one apps crate
depending on an apps crate of another family, so the hub cannot depend on
`rusty_multimodal_db`. The store has to live in the `libs` layer.

The generic store (`src/generic/`) was almost self-contained. Its only
ties to the rest of this crate were:

- `DurabilityError`, which it returns everywhere, has a `Store` variant
  wrapping the Dog `StoreError`;
- the record blob header, fingerprint hash and atomic install it shares
  with the Dog `RecordBlob`, which lived in `durability/record_blob.rs`
  next to that Dog type;
- `codec`, the one bincode configuration;
- test fixtures: the `Order` domain (`research`), the Dog blob and the
  research `Employee` type.

## Decision

Move the generic store into a new crate,
`crates/libs/storage/rusty_multimodal_db_engine` (`layer = "libs"`), and
have this crate depend on it and re-export it under the paths it had.

- **Moved:** `generic/{traits, store, mmap_field, mmap_store,
  mmap_scanned, production, query, slot_file, insert_log, record_blob,
  edge_blob, order_customer}` and `generic/mod.rs`'s own items (the
  error types, `CompactionReport`, `LinkOutcome`, …); `codec`;
  `DurabilityError` and `sync_parent_dir`; and the shared half of
  `durability/record_blob.rs` (`HEADER_LEN`, `Fnv1a64`, `parse_header`,
  `encode_image`, `companion_path`, `EncodedRecordBlob`).
- **Stayed:** this crate's domains (`generic/{entity, memory, relation,
  reminder}`), the Dog store and every durability variant, the Dog
  `RecordBlob`, the server and the research spikes.
- **Paths are unchanged.** `generic/mod.rs` now glob re-exports the
  engine's `generic` and declares only the domains, so
  `crate::generic::store`, `crate::generic::GenericMmapStore` and the rest
  resolve as before. `durability` re-exports `DurabilityError`, and `lib.rs`
  re-exports `codec`.
- **`DurabilityError::Store` holds a boxed error.** The engine cannot name
  the Dog `StoreError`. This crate implements `From<StoreError> for
  DurabilityError` (legal, since `StoreError` is local), so every `?` on a
  `StoreError` still converts. The one place that read the variant back,
  `From<DurabilityError> for StoreError`, downcasts. No other code matched
  on it.
- **Visibility.** Modules this crate's domains reach into (`slot_file`,
  `insert_log`, `record_blob`, `edge_blob`, `store::normalize`, `codec`'s
  functions) became `pub` and `#[doc(hidden)]`: an implementation surface
  for the one crate that shares it, not a supported API.
- **The engine's tests stand alone.** The three tests that wrote a real
  Dog blob at a generic blob's path now write a blob with the Dog magic
  (`DOGBLOB\0`, version 2) through the shared `encode_image`. The engine
  refuses it on magic, which is all they checked. The two that used the
  research `Employee` type carry a four-impl copy of it with the same
  schema tag. `Order` moved with the engine under a `research` feature,
  which this crate's `research` enables.
- **Dependencies are the pins this crate already used** (`uuid`, `serde`,
  `bincode` 1, `memmap2` 0.9, `thiserror` 1), kept literal: the blobs and
  insert log are bincode on-disk formats, so a dependency drifting under
  them would change bytes already written.

## Consequences

- `rusty_remind_me`'s hub can depend on `rusty_multimodal_db_engine`
  without breaking the layer rule. That is phase 2 of its ADR-0021.
- This crate's public API is unchanged: every public path resolves to the
  same item, now defined in the engine crate.
- The engine carries `research` only for the `Order` fixture, which its
  own tests and this crate's research benches and order server use.
- Changes to the generic store now land in the engine crate. The file
  moves kept their history (`git mv`).

## Alternatives declined

- **Move all of `rusty_multimodal_db` to libs.** It is a 64k-line server
  with binaries and research code, which the libs layer is not for.
- **An allow-list exception in the layer check.** Quicker, but it weakens
  a rule the whole workspace relies on.
- **Split out the network client instead (`ADR-0043`'s declined option).**
  `rusty_remind_me` chose to embed the store (its ADR-0021), so the client
  split would not unblock it.
