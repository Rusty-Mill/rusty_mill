# Nexus / `remind_me` sync contract evidence (#445)

**Scope:** evidence only; no migration, consolidation, runtime wiring, schema,
or owner-policy change. Fixtures are synthetic and contain no user data,
secrets, or HTTP authorization headers. They illustrate JSON contracts from
source inspected at baseline `99d41507737523d1e5412a2545c4825f1fb45701`.
They are hand-authored contract illustrations, not captured traffic or proof
that a cross-product request or a lossless round trip completed successfully.

PR #491 (`a2061b02b76cd99460f5c4612adeb7a8400cd0b0`) merged on 2026-10-04.
Its schema-v32 context/provenance fields are included in this refresh. The
current `remind_me_core` store is unconditionally engine-backed; SQLite is
retained for legacy reads/import, not as an alternate runtime store.

## Executable samples

The four files in [`fixtures/`](fixtures/) capture representative serialized
push bodies and pull request/response shapes. Run
`python3 docs/research/memory-sync-contract/check_fixtures.py` to verify the
shared envelope and the deliberately exposed contract gaps. `Z` versus
`+00:00` in the samples represents two legal RFC 3339 UTC spellings, not a
semantic mismatch.

The checker also requires all 13 v32 fields in both remind_me samples, equal
values between those samples, and their absence from the Nexus record sample.
It does not execute Rust serializers, HTTP requests, or backend migrations.
The remind_me pull sample illustrates the engine-backed hub's emptied
tombstone shape; it does not assert parity with the Postgres backend.

## Source-linked compatibility matrix

| Contract surface | Nexus on `main` | `remind_me` on `main` | Compatibility |
|---|---|---|---|
| Push route/envelope/reply | `POST /sync/push`; `{node_id, records}`; advances only from the reply's `processed_ids` (`nexus-memory/src/sync.rs:377-430`) | Same request and per-ID acknowledgement (`remind_me_core/src/sync/push.rs:131-176`) | **Wire-compatible envelope.** Record semantics below are not. |
| Pull route/envelope | `GET /sync/pull?since=&since_id=&exclude_node=&limit=` and `{records,count}` (`nexus-memory/src/sync.rs:477-507`) | Same timestamp/id fallback; hubs can additionally return per-record `hub_seq`, selected with `since_seq` (`remind_me_core/src/sync/pull.rs:102-173,196-299`) | **Common fallback; capability mismatch.** A sequence cursor cannot round-trip through Nexus. |
| Timestamps | Strong `DateTime<Utc>` model, serialized as RFC 3339 (`nexus-memory/src/model.rs`, `Memory`) | Strings on both domain and sync DTOs (`remind_me_core/src/models.rs`, `Memory`; `sync/record.rs`, `SyncRecord`) | Same typical wire spelling is accepted, but `remind_me`'s DTO does not give the type-level validation Nexus has. Offset-equivalent strings also affect string-ordered LWW/cursors unless canonicalized at the boundary. |
| Shared record fields | 23 serialized fields including capture/SPO/vitality and `client`, `node_id`, `source_capture_id` (`nexus-memory/src/model.rs`, `Memory`) | The sync DTO carries that shared set plus 16 fields: deletion/reminder/sensitivity and 13 v32 values ([node DTO][node-record]) | **Near**, not identical. Identity fields are not Nexus-only. There is no defined translation for identity ownership. |
| Context/provenance (v32) | No dedicated fields for the 13 values below in `Memory` | `Memory`, `SyncRecord`, `SyncRecord::from_memory`, `upsert_record`, and the engine-backed hub carry them ([node DTO][node-record], [hub row][hub-row]) | **Lossy without an explicit extension.** Generic metadata is not an implemented mapping; a Nexus round trip is not proven to preserve these fields. |
| Tombstone | `status = "deleted"`; status is a closed enum (`nexus-memory/src/model.rs:142-183`) | `deleted_at` is the deletion marker; `status` is a free string ([node DTO][node-record]) | **Incompatible semantics.** Passing either representation unchanged can resurrect or fail to hide a deletion. |
| Memory type | Closed `episodic`, `semantic`, `procedural`, `unclassified`; unknown DB values collapse to `unclassified` (`nexus-memory/src/model.rs:108-140`) | Free string/default `unclassified`; product values include `decision`, `fact`, and others (`remind_me_core/src/sync/record.rs:28-38,121-125`) | **Lossy.** Nexus cannot preserve an open vocabulary round trip. |
| Reminder/sensitivity | No fields in `Memory` (`nexus-memory/src/model.rs:209-263`) | `remind_at` and `sensitive` are transmitted and applied ([node DTO][node-record], `upsert_record`) | **Lossy.** A Nexus round trip drops behavior and visibility intent. |
| Document chunk identity | No `doc_id` or `chunk_index` | Stored in the domain/schema, but intentionally absent from the receiving `SyncRecord` (`remind_me_core/src/models.rs:82-85`; `sync/record.rs:69-82`) | **Not preserved by today's sync contract on either cross-product path.** Do not call a database conversion reversible. |
| Change capture/conflict | Timestamp/id scan pushes only locally-authored rows; foreign edits are a documented v1 limitation (`nexus-memory/src/sync.rs:298-328`) | Store-maintained outbox, per-remote send tracking; record apply unions tags and shallow-merges metadata around LWW (`remind_me_core/src/sync/push.rs`; [node DTO][node-record], `upsert_record`) | **Incompatible delivery and merge semantics.** Matching JSON does not imply matching convergence. |
| Optional dependency closure | `nexus-memory` has no equivalent content-ingestion feature set and already depends on Nexus kernel/plugins (`nexus-memory/Cargo.toml`) | `default = []`; the engine dependency is mandatory and `engine-store` no longer exists. PDF, cloud backup, ANN, OCR, audio, reranking, local embeddings, and stack dumps remain feature-gated ([manifest][core-manifest]) | Embedding still introduces an app-to-app boundary and unrelated storage/feature choices. The earlier 65/72-line default-feature measurements are obsolete. |

The 13 fields are `project`, `session_id`, `git_remote`, `git_branch`,
`git_sha`, `cwd`, `valid_from`, `valid_until`, `confidence`, `verified_at`,
`outcome`, `written_by`, and `capture_method`. Missing values deserialize as
`None` except `confidence = 1.0`, `written_by = "unknown"`, and
`capture_method = "manual"`. These compatibility defaults do not preserve
values discarded by a peer. The node's existing
`the_v32_columns_round_trip_through_the_outbox_and_apply` test is evidence
for its own outbox/apply path, not for Nexus or every hub backend.

[node-record]: ../../../crates/apps/rusty_remind_me/crates/remind_me_core/src/sync/record.rs
[hub-row]: ../../../crates/apps/rusty_remind_me/crates/remind_me_hub/src/store/multimodal/rows.rs
[core-manifest]: ../../../crates/apps/rusty_remind_me/crates/remind_me_core/Cargo.toml

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
8. **Context/provenance is not shared yet.** All 13 v32 fields need explicit,
   lossless representation and conflict rules; silently defaulting a value
   after a peer drops it is not preservation.

## Approved architecture and remaining design work

**Keep Nexus and remind_me stores separate.** This owner decision is approved.
Do not embed `remind_me_core`: the
schemas and domain semantics do not match, and an app-layer dependency would
carry unrelated storage and optional feature choices. Do not share the current
sync implementation merely because the routes match: the tombstone, type,
conflict, change-capture, and cursor contracts do not.

The approved direction is to design **lossless common sync context/provenance
fields before consolidation**. This evidence does not implement that contract:
field ownership, null/default behavior, conflict rules, version negotiation,
and preservation of the other semantic gaps above still need a specification
and executable cross-product round-trip tests. No migration starts from these
fixtures, and no decision is requested again here.

The related MCP boundary is also approved: the reusable protocol client belongs
in `rusty_mcp`, with Nexus owning configuration and policy adapters. That does
not authorize coupling either memory store to the other.
