# Nexus / `remind_me` sync contract evidence (#445)

**Scope:** evidence only; no migration, consolidation, runtime wiring, schema,
or owner-policy change. Fixtures are synthetic and contain no user data,
secrets, or HTTP authorization headers. They model the JSON emitted by the
source at baseline `69fc7bdd4fbe4c6fdb8e79f24d2bff070c0ca4cf`; they are not a claim that
a cross-product request completed successfully.

PR #491 (`a2061b02b76cd99460f5c4612adeb7a8400cd0b0` when checked) proposes later
`remind_me` schema/store changes. It is open, unmerged, and **not authority for
this study**. Every finding below describes current `main`; #491 must be
re-evaluated if it lands.

## Executable samples

The four files in [`fixtures/`](fixtures/) capture representative serialized
push bodies and pull request/response shapes. Run
`python3 docs/research/memory-sync-contract/check_fixtures.py` to verify the
shared envelope and the deliberately exposed contract gaps. `Z` versus
`+00:00` in the samples represents two legal RFC 3339 UTC spellings, not a
semantic mismatch.

## Source-linked compatibility matrix

| Contract surface | Nexus on `main` | `remind_me` on `main` | Compatibility |
|---|---|---|---|
| Push route/envelope/reply | `POST /sync/push`; `{node_id, records}`; advances only from the reply's `processed_ids` (`nexus-memory/src/sync.rs:377-430`) | Same request and per-ID acknowledgement (`remind_me_core/src/sync/push.rs:131-176`) | **Wire-compatible envelope.** Record semantics below are not. |
| Pull route/envelope | `GET /sync/pull?since=&since_id=&exclude_node=&limit=` and `{records,count}` (`nexus-memory/src/sync.rs:477-507`) | Same timestamp/id fallback; hubs can additionally return per-record `hub_seq`, selected with `since_seq` (`remind_me_core/src/sync/pull.rs:102-173,196-299`) | **Common fallback; capability mismatch.** A sequence cursor cannot round-trip through Nexus. |
| Timestamps | Strong `DateTime<Utc>` model, serialized as RFC 3339 (`nexus-memory/src/model.rs:209-263`) | Strings on both domain and sync DTOs (`remind_me_core/src/models.rs:60-116`; `sync/record.rs:83-159`) | Same typical wire spelling is accepted, but `remind_me`'s DTO does not give the type-level validation Nexus has. Offset-equivalent strings also affect string-ordered LWW/cursors unless canonicalized at the boundary. |
| Shared record fields | 24 serialized fields including capture/SPO/vitality and `client`, `node_id`, `source_capture_id` (`nexus-memory/src/model.rs:209-263`) | The sync DTO carries that shared set (`remind_me_core/src/sync/record.rs:83-159`) | **Near**, not identical. The issue's statement that identity fields are Nexus-only is stale on this baseline. There is no defined translation for identity ownership. |
| Tombstone | `status = "deleted"`; status is a closed enum (`nexus-memory/src/model.rs:142-183`) | `deleted_at` is the deletion marker; `status` is a free string (`remind_me_core/src/sync/record.rs:121-145`) | **Incompatible semantics.** Passing either representation unchanged can resurrect or fail to hide a deletion. |
| Memory type | Closed `episodic`, `semantic`, `procedural`, `unclassified`; unknown DB values collapse to `unclassified` (`nexus-memory/src/model.rs:108-140`) | Free string/default `unclassified`; product values include `decision`, `fact`, and others (`remind_me_core/src/sync/record.rs:28-38,121-125`) | **Lossy.** Nexus cannot preserve an open vocabulary round trip. |
| Reminder/sensitivity | No fields in `Memory` (`nexus-memory/src/model.rs:209-263`) | `remind_at` and `sensitive` are transmitted and applied (`remind_me_core/src/sync/record.rs:127-159,286-305`) | **Lossy.** A Nexus round trip drops behavior and visibility intent. |
| Document chunk identity | No `doc_id` or `chunk_index` | Stored in the domain/schema, but intentionally absent from the receiving `SyncRecord` (`remind_me_core/src/models.rs:82-85`; `sync/record.rs:69-82`) | **Not preserved by today's sync contract on either cross-product path.** Do not call a database conversion reversible. |
| Change capture/conflict | Timestamp/id scan pushes only locally-authored rows; foreign edits are a documented v1 limitation (`nexus-memory/src/sync.rs:298-328`) | Triggered outbox, per-remote send tracking; record apply unions tags and shallow-merges metadata around LWW (`remind_me_core/src/sync/push.rs:1-18`; `sync/record.rs:1-8,183-284`) | **Incompatible delivery and merge semantics.** Matching JSON does not imply matching convergence. |
| Optional dependency closure | `nexus-memory` has no equivalent content-ingestion feature set and already depends on Nexus kernel/plugins (`nexus-memory/Cargo.toml`) | Default enables `engine-store`; PDF, cloud backup, ANN, OCR, audio, reranking, local embeddings, and stack dumps are feature-gated (`remind_me_core/Cargo.toml`) | Embedding is not a small sync reuse: it introduces an app-to-app boundary and a substantially different dependency surface. Measure with the commands below. |

## Concrete round-trip and migration risks

1. **Deletes can resurrect.** Nexus sends a status tombstone while
   `remind_me` acts on `deleted_at`; neither marker is a lossless synonym
   without an explicit mapping and tests.
2. **Types collapse.** A `decision`/`fact` sent through Nexus can return as
   `unclassified`; treating this as success loses product behavior.
3. **Reminder and sensitivity intent disappears.** Nexus has nowhere to retain
   `remind_at` or `sensitive`.
4. **Chunk provenance disappears.** `doc_id` and `chunk_index` are not in the
   receiving sync DTO, so even a `remind_me`-only wire round trip does not prove
   database round-trip preservation.
5. **Delivery differs.** Nexus does not propagate local edits to foreign-authored
   rows, whereas the outbox is authorship-agnostic. Cursor compatibility cannot
   repair this convergence difference.
6. **Conflict results differ.** Tag union and metadata merge can produce a row
   neither whole-row LWW side sent. Cross-product replay is therefore not
   proven idempotent.
7. **Identity has no shared policy.** Similar fields exist on both sides, but
   `client`, author `node_id`, capture IDs, and `origin_node` transport meaning
   must be specified before adapters rewrite them.

## Recommendation and owner decision

**Keep both implementations for now.** Do not embed `remind_me_core`: the
schemas and domain semantics do not match, and an app-layer dependency would
carry unrelated storage and optional feature choices. Do not share the current
sync implementation merely because the routes match: the tombstone, type,
conflict, change-capture, and cursor contracts do not.

A future **shared sync-only contract** is the least invasive consolidation
option, but only after the owner makes one precise decision:

> Must the shared contract losslessly preserve `remind_me`'s open memory types,
> `deleted_at`, reminders, sensitivity, and chunk identity, or may it reject
> records containing semantics the receiver cannot represent?

Until that choice is explicit, refusal is safer than silent coercion. No
migration should begin from these fixtures, and #491 should receive a fresh
comparison only after it merges.
