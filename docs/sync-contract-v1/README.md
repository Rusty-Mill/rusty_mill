# Memory sync compatibility contract v1 (draft)

Status: **Draft for issue #445; not adopted by either product**  
Source baseline: `eb04ffdf614a4868459fbde336620f0de856b33e`

## Decision and boundary

Nexus and `remind_me` remain separate stores and applications. This proposal
defines a versioned compatibility boundary between them; it does **not** select
either product as canonical domain owner or canonical store. It also does not
authorize database migration, deletion, consolidation, or production sync
rewiring.

Directly embedding `remind_me_core` in Nexus is not proposed. Both are app-layer
crates, and the workspace layering rule forbids dependencies between application
families. More importantly, embedding code would not resolve incompatible
deletion, memory-type, conflict, reminder, and sensitivity semantics. Migration
is also deferred: the existing import path is not a reversible conversion and
neither forward nor rollback preservation has been demonstrated.

This draft is based on source analysis. The fixtures are synthetic examples and
the validator is a hand-written model of this proposal. They are **not captured
successful HTTP round trips** and do not prove full legacy interoperability.
No real `.forge` or `remind_me` database was inspected.

The source reconciliation used the current Nexus memory model and node sync
path (`crates/apps/nexus/crates/nexus-memory/src/model.rs` and `sync.rs`), the
Nexus hub (`crates/apps/nexus/crates/nexus-memory-hub/src/lib.rs`), the current
`remind_me` sync DTO/conflict implementation
(`crates/apps/rusty_remind_me/crates/remind_me_core/src/sync/record.rs`), its
push/pull clients (`sync/push.rs` and `sync/pull.rs`), hub routes
(`crates/apps/rusty_remind_me/crates/remind_me_hub/src/routes.rs`), and database
schema (`remind_me_core/src/db/schema_tables.sql`). The database-only
chunk/document distinction is also visible in `SyncRecord`: those columns are
not serialized by that DTO.

## Protocol envelope and negotiation

Every request carries this envelope:

```json
{
  "contract": "rusty-mill.memory-sync",
  "version": 1,
  "capabilities": ["cursor.timestamp-id", "memory-types.closed", "tombstone.status"],
  "node_id": "opaque-node-id",
  "body": {}
}
```

`contract`, `version`, `capabilities`, `node_id`, and `body` are required.
Unknown envelope fields may be retained by transports but have no meaning in
v1. A peer must reject an unsupported version before processing records.
Negotiation is the exact intersection of advertised capabilities; successful
hub storage is never evidence that a receiving node can apply a record.

### Capabilities

| Capability | Meaning |
|---|---|
| `cursor.timestamp-id` | Stable pagination by the pair (`updated_at`, `id`). |
| `cursor.sequence` | Stable pagination by a hub-issued opaque sequence cursor. |
| `memory-types.closed` | Only `episodic`, `semantic`, `procedural`, and `unclassified`. |
| `memory-types.open` | Arbitrary non-empty type strings can be preserved. |
| `tombstone.status` | Deletion is `status: "deleted"`. |
| `tombstone.deleted-at` | Deletion is a non-null `deleted_at`. |
| `field.sensitive` | Boolean sensitivity is preserved end to end. |
| `field.remind-at` | Nullable reminder timestamp is preserved end to end. |
| `field.lineage` | Authorship, capture, supersession, and SPO lineage is preserved. |

A sender must not send a semantic feature unless the receiver advertises a
lossless capability for it. Where representations differ (notably tombstones),
an adapter may translate only after both sides explicitly advertise the two
representations and the deployment selects a mapping policy. Otherwise the
record is refused, never coerced or partially applied.

## Operations

### Push

`body` is `{ "records": [record, ...] }`. The response body is:

```json
{
  "processed_ids": ["id-a"],
  "refused": [{"id": "id-b", "code": "unsupported_memory_type", "detail": "fact"}]
}
```

For every distinct requested ID there must be exactly one outcome: once in
`processed_ids`, or once in `refused`. Duplicate request IDs are malformed.
Missing outcomes, duplicate outcomes, and an ID appearing in both sets make the
entire response invalid; clients must not mark any unaccounted record delivered.
`accepted`/`failed` counts, if exposed by a legacy adapter, are informational and
cannot replace per-ID accounting.

### Pull

The request body is `{ "cursor": cursor-or-null, "limit": integer }`, where
`limit` is 1 through 1000. The response body is:

```json
{"records": [], "next_cursor": null, "has_more": false}
```

The cursor is opaque to the client and is valid only with the negotiated cursor
capability. A peer must refuse `cursor.sequence` versus `cursor.timestamp-id`
mismatch rather than interpreting one as the other. If `has_more` is true,
`next_cursor` is required. If false, `next_cursor` may be null. V1 defines no
implicit default cursor capability.

## Record schema

JSON objects use the following exact v1 names and types. Required nullable
fields must be present even when null; this avoids asymmetric serde defaults.

| Field | Type | Rule |
|---|---|---|
| `id` | string | Required, opaque, 1–128 ASCII alphanumeric/`_`/`-`; never parse prefixes. |
| `content` | string | Required. Tombstone content policy is negotiated; live content is non-empty. |
| `category` | string | Required, non-empty. |
| `tags` | array of strings | Required; defaulting by an adapter is forbidden. |
| `source` | string | Required, non-empty. |
| `metadata` | object | Required. |
| `created_at` | RFC 3339 string | Required, UTC-normalizable. |
| `updated_at` | RFC 3339 string | Required, UTC-normalizable. |
| `client` | string | Required. |
| `node_id` | string or null | Required; opaque authorship identity. |
| `capture_id` | string or null | Required lineage. |
| `source_capture_id` | string or null | Required lineage. |
| `memory_type` | string | Required; capability controls vocabulary. |
| `status` | string | Required; v1 common live value is `active`. |
| `deleted_at` | RFC 3339 string or null | Required; semantics depend on tombstone capability. |
| `sensitive` | boolean | Required; requires `field.sensitive` when true. |
| `remind_at` | RFC 3339 string or null | Required; requires `field.remind-at` when non-null. |
| `superseded_by` | string or null | Required opaque ID; requires `field.lineage` when non-null. |
| `subject`, `predicate`, `object` | string or null | Required; require `field.lineage` when any is non-null. |
| `accessed_at` | RFC 3339 string or null | Required. |
| `access_count` | non-negative integer | Required. |
| `decay_rate`, `vitality`, `base_weight` | finite number | Required. |

There are no v1 defaults. A legacy adapter may construct an explicit v1 record
from absent legacy fields only under a separately approved mapping policy; it
must not describe that as lossless preservation.

IDs remain opaque even though v1 narrows their wire alphabet to the intersection
currently accepted by Nexus. A future contract version can negotiate a wider ID
syntax. Malformed IDs are refused as `invalid_id`, not rewritten.

Timestamps must parse as RFC 3339, denote a real instant, and include an offset.
Comparison uses the normalized instant, not its spelling. Implementations may
apply a documented future-skew limit, but must return `invalid_timestamp` rather
than silently clamp. `created_at` is insert-only.

## Conflict and tie semantics

V1 uses strict last-write-wins on normalized `updated_at`. A newer incoming
record replaces the stored record as a whole. An older record loses. Equal-time
records are **refused with `equal_time_conflict` unless their canonical v1 JSON
is identical**, in which case the operation is an idempotent success.

This deliberately does not choose Nexus whole-record tie behavior or
`remind_me`'s tag-union/metadata-shallow-merge behavior. Automatic merging on an
equal or losing timestamp would choose a canonical product semantic, so it is an
unresolved future capability (for example `conflict.merge-tags-metadata`). Until
then a disagreement is explicit and deterministic.

## Preservation and refusal rules

* Open memory types cannot be sent to `memory-types.closed` unless the value is
  one of its four declared values. Refuse `unsupported_memory_type`; never map
  `fact` or `decision` to `unclassified` silently.
* A true `sensitive` value without `field.sensitive`, or a non-null `remind_at`
  without `field.remind-at`, is refused. False/null values are representable but
  the receiver must still retain the explicit fields in a v1 record.
* Non-null capture, supersession, or SPO lineage without `field.lineage` is
  refused. Emptying these fields is not compatibility.
* `status: "deleted"` and non-null `deleted_at` are different deletion markers.
  A mismatch is refused as `tombstone_capability_mismatch` unless an explicitly
  selected adapter policy translates it. Live records require `deleted_at:null`.
* Document `doc_id` and `chunk_index` are database-only `remind_me` fields, not
  v1 memory-sync fields. Their absence from this record is intentional. A
  database migration must preserve them separately; sync must not claim to do
  so.

## Existing-product mapping matrix

| Boundary | Nexus | `remind_me` | V1 result |
|---|---|---|---|
| Envelope | Legacy `{node_id,records}` | Same superficial shape | Adapter must add/version/negotiate v1. |
| Required fields | Complete `Memory` shape | Four required; many serde defaults | V1 requires all fields; sparse records refused or explicitly adapted. |
| IDs | Validated opaque strings; UUIDv7 common | Opaque strings; `mem_…` common | Shared intersection above; no prefix semantics. |
| `client`, `node_id`, `source_capture_id` | Present | **Also present now** (older issue text was stale) | Direct field mapping. |
| Capture/SPO/supersession/vitality | Present | Present on current sync record | Preserve with `field.lineage` where applicable. |
| Memory type | Closed enum | Open string | Capability-gated; unsupported values refused. |
| Deletion | Closed `status`, including `deleted` | Non-null `deleted_at`; status alone is not deletion | No implicit conversion; adapter policy unresolved. |
| Sensitivity/reminder | No model fields | `sensitive`, `remind_at` | Nexus-facing peer must refuse meaningful values until it can preserve them. |
| Equal timestamp | Whole record does not win | Losing record can still merge tags/metadata | V1 refuses non-identical ties; future merge capability unresolved. |
| Remote-authored edits | Push filtering by node ownership | Outbox delivers local edits per remote | Ownership policy unresolved; advertise/refuse until selected. |
| Push reply | Processed IDs plus structured rejections | Processed IDs; legacy fallback can mark a page sent | V1 requires complete per-ID outcomes; no legacy success fallback. |
| Pagination | Timestamp/ID | Timestamp/ID plus sequence/full reseed | Negotiate one cursor capability per session. |
| Hub storage | Can accept opaque JSON after limited checks | Parses a broader record | Storage acceptance is not node compatibility. |
| Chunks/documents | Not memory fields | `doc_id`/`chunk_index` exist in DB but are excluded from current sync DTO | Outside v1; migration-only preservation obligation. |

## Adoption, migration, and rollback prerequisites

Adoption requires independent review of this contract, implementation adapters
on both clients and hubs, capability downgrade tests, authentication/transport
review, and actual cross-family HTTP probes in all four client/hub directions.
Those probes must cover mixed batches, pagination, concurrent conflicts, and
restart/retry behavior and must distinguish hub acceptance from node apply.

Before any migration, owners must select canonical domain ownership and define
lossless forward and reverse mappings for tombstones, open types, reminders,
sensitivity, lineage, chunks/documents, auxiliary tables, and indexes. Migration
requires immutable backups, dry-run inventories, row/ID/semantic reconciliation,
version gates, dual-read or quiescence policy, and a tested rollback exporter.
Rollback after new writes must preserve every v1 field and product-only data.
The current importer is not that proof.

## Fixture evidence and limits

`fixtures/cases.json` contains only invented values. Run:

```sh
python3 -m unittest discover -s docs/sync-contract-v1 -p 'test_*.py' -v
```

The suite checks the normative validation and negotiation rules above: a
compatible record, both deletion-marker mismatches, open-to-closed memory types,
sensitivity/reminder/lineage preservation, malformed timestamps and IDs,
equal-time conflicts, incomplete push accounting, and cursor mismatches. It
does not import product crates, start either HTTP server, or exercise either
legacy decoder. Passing it proves fixture consistency with this draft only.
