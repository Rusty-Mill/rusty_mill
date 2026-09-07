# Hub spike report — can `rusty_multimodal_db` back `rusty_remind_me`'s `HubStore`? (2026-09-07)

Provenance: written by an integration spike run against this crate at
`1e80b41` and the consumer at `593f793`, in a separate session without
push access; the consumer-side branch is `claude/multimodal-hub-spike`
(one commit, handed over as a patch). Kept here verbatim, below the
line, because it is the evidence the next rounds in the
`rusty_remind_me` line are chosen from — see `docs/PROJECT-STATUS.md`
item 158 and `docs/design/SERVER-DATA-DIR-DESIGN.md`'s open questions.

Re-verified in this repository's session before recording: the patch
applied on the consumer's `main`, built under its pinned 1.97 toolchain
with the `multimodal` feature, and all ten tests passed against the
**shipped** `memory_server` at `e592e46` (`ADR-0053`) started with
`SERVER_DATA_DIR` on an empty directory — the custom fixed-path binary
the report describes is no longer needed. Clippy `-D warnings` clean;
the test file needed the pinned rustfmt applied. Two of the report's
findings were already acted on before it arrived: the column count
(`ADR-0048` erratum, PR #209) and the binary's scratch directory
(`ADR-0053`, PR #210).

What the report settles for this crate, in the order the rounds should
go: (1) a guarded replace for the hub's last-writer-wins; (2) a
range-scannable `updated_at_unix_ms` and a bounded ordered page for the
pulls; (3) sync bookkeeping in the projection — `deleted_at` and
`node_id`, which needs the null story; (4) directed open-label edges
for `entity_relations`; a global edge count as an aggregate whenever a
round touches relations. The entity-id encoding is the consumer's
decision (`entity_id` here is the untruncated form of its own).

---

# Spike report: can `rusty_multimodal_db` back `rusty_remind_me`'s `HubStore`?

**Answer: partially, with real gaps.** Everything below is from an actual live server
and an actual compiled, clippy-clean, test-passing implementation — not a design
read-through. Repos: `baileyrd/rusty_multimodal_db` @ `1e80b41` (main), `baileyrd/rusty_remind_me`
@ `593f793` (main), branch `claude/multimodal-hub-spike`.

## Sandbox constraint, stated once

This ran in an ephemeral Ubuntu container with no GitHub credentials (no `gh`, no SSH
key, no PAT). Everything below — clone, build, run, test — was done for real. The one
step I could not do is push the branch or open the PR. A ready-to-apply patch and exact
push commands are at the bottom.

Both repos needed a newer Rust than the container's default `apt` `rustc` (1.75):
`rusty_multimodal_db`'s dependency tree needs ≥1.85, `rusty_remind_me`'s
`rust-toolchain.toml` pins 1.97. Installed `rustc-1.91`/`cargo-1.91`/`rustfmt-1.91`/
`rust-1.91-clippy` via `apt` (Ubuntu 24.04's universe repo carries them) and forced
`RUSTC=/usr/bin/rustc-1.91` explicitly — plain `rustc` on `PATH` stays 1.75, so any
`cargo` invocation that doesn't set this silently uses the wrong compiler and fails on
newer transitive deps (`zeroize` etc.). Both workspaces build clean under 1.91 despite
`rusty_remind_me` nominally wanting 1.97.

---

## Phase 1 — smoke test

### 1. Protocol / build

`memory_server` (`cargo build --release --features server --bin memory_server`) builds
and negotiates `Hello { 18 }` — matches the task brief exactly.

**Finding, not in the brief:** the shipped `memory_server` binary reseeds fresh sample
data into a new per-PID temp directory (`std::env::temp_dir().join(format!("memory_server_{}",
process::id()))`) on *every* launch — it cannot demonstrate cross-restart persistence by
itself. To actually prove persistence (step 2 below), I wrote a ~90-line alternate binary
(`persistent_server`, included in this report's attachments) that opens a fixed path via
`open_memory_production_stack_portable`/`open_entity_production_stack_portable` if it
exists, or creates fresh if not — same domains, same wire shape, just a stable path.

### 2. Full write path, over a live socket — all steps passed

insert memory → insert entity → link `mentions` → whole-record `replace` (content verified
changed) → `JOIN memory m ON entity e ON mentions` (1 row) → delete entity → same join now
0 rows (cascade confirmed) → `compact` (`records=2, edge_logs_folded=1`, counts consistent).

Two field-name corrections needed along the way, both now-obvious but worth noting since
the task brief's field names didn't match current `main`: `Entity`'s name field is `label`,
not `name`; entity `insert` requires `mention_count` explicitly (it's not optional/derived
at insert time).

**Restart-and-reopen, actually run:** killed the server process, confirmed via `ps` it was
gone, confirmed every `.mmap`/`.records`/`.edges`/`.relations`/`.inserts` file was still on
disk, relaunched against the same directory (log line: `reopening existing memory store`,
not `creating new`), and a fresh client connection read back exactly 2 memories (seed +
ours, with the **post-replace** content) and exactly 1 entity (only the seed — ours was
deleted before the restart). Insert, Replace, Delete-with-cascade, and Compact all survived
a real process restart.

### 3. Column mapping — a real discrepancy found

The task brief and `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md` both say "the consumer's
thirty-column `memories` table." I parsed `crates/remind_me_core/src/db/schema_tables.sql`
(the exact file the design doc cites) programmatically rather than counting by eye:

**it currently has 28 columns, not 30.**

Full list: `id, content, category, tags, source, metadata, created_at, updated_at,
capture_id, node_id, client, accessed_at, access_count, decay_rate, vitality, base_weight,
status, memory_type, source_capture_id, subject, predicate, object, superseded_by, doc_id,
chunk_index, deleted_at, remind_at, sensitive`.

Against `rusty_multimodal_db`'s eleven-field `Memory` projection (`content`, `category`,
`tags`, `source`, `metadata_json`↔`metadata`, `created_at_unix_ms`↔`created_at`,
`updated_at_unix_ms`↔`updated_at`, `memory_type`, `status`, `sensitive`, `access_count`,
plus `id` itself as the record's identity — 12 of the 28 have a home):

**16 columns with no home**, not 19 as the design doc's "nineteen omitted" count claims
(that figure likely also counted the two edge tables, `memory_entities`/`memory_associations`,
which aren't columns of `memories` at all — and predates whatever removed 2 columns from
the live schema since the design doc was written):

`capture_id, node_id, client, accessed_at, decay_rate, vitality, base_weight,
source_capture_id, subject, predicate, object, superseded_by, doc_id, chunk_index,
deleted_at, remind_at`

No `embedding` column exists on `memories` at all (there's a separate `memories_fts` FTS5
virtual table, not a column) — the design doc's "`embedding`, full-text search" bullet
refers to search infrastructure in general, not a specific column, worth clarifying if that
doc is revised.

---

## Phase 2 — `HubStore` spike (`MultimodalHubStore`)

Branch `claude/multimodal-hub-spike` on `rusty_remind_me`, `multimodal` cargo feature
(off by default), `rusty_multimodal_db` pinned as a git dependency (`rev = 1e80b416...`),
`features = ["client"]` only. `cargo fmt --check` clean, `cargo clippy --all-targets -D
warnings` clean on both the `multimodal` feature set and the default set, default build
unaffected. 10/10 integration tests pass against a live server
(`crates/remind_me_hub/tests/hub_multimodal_test.rs`).

### Method-by-method

| # | `HubStore` method | Result | Why |
|---|---|---|---|
| 1 | `migrate` | ✅ pass (no-op) | Schema is compiled into the server binary, not client-issued DDL — nothing to create |
| 2 | `ping` | ✅ pass | `GetById(nil)` on each connection — cheapest real round trip available; no dedicated no-op request |
| 3 | `apply_record(Memory)` | ✅ pass | Insert-or-LWW-replace, done client-side (get, compare `updated_at_unix_ms`, insert/replace/no-op) |
| 3 | `apply_record(Entity)` | ✅ pass, simplified | Insert-or-replace unconditionally (no `updated_at` field exists to compare); preserves existing `mention_count` across a replace rather than resetting it |
| 3 | `apply_record(Link)` | ✅ pass | `link()` is insert-or-ignore, matching `memory_entities`'s own `ON CONFLICT DO NOTHING` almost exactly |
| 3 | `apply_record(EntityRelation)` | ❌ **fails, by design** | **No backend primitive at all** — see gaps below. Returns `Err`, not a silent no-op, so a push surfaces it as a real failure rather than quietly losing data |
| 4 | `stats` | ✅ pass, with named defaults | `tombstones: 0` (no `deleted_at` concept), `by_origin_node` is `[(UNATTRIBUTED, total)]` or `[]` (no origin tracking), `entity_relations: 0` |
| 5 | `count_tables` | ✅ pass | `memories`/`entities` are real full-table counts; `memory_entities` via per-record `neighbors_by_relation` sum; `entity_relations` always `0` |
| 6 | `approx_count_tables` | ✅ pass (`Ok(None)` always) | No planner statistics of any kind on this backend — matches the brief's own prediction |
| 7 | `count_tables_since` | ✅ pass, degraded for entities | `memories` filters client-side (no server-side range filter — see gaps); `entities` has no timestamp at all, so it returns the **unconditional total** rather than 0 or an error, documented as the safer overestimate |
| 8 | `count_by_origin_node` | ✅ pass (`[]` always) | No per-record origin tracked at all |
| 9 | `count_by_category` | ✅ pass, genuinely server-side | `SELECT category, COUNT(*) FROM memory GROUP BY category` is a real query — `category` is the one groupable field `Memory` has. `since`-filtered variant falls back to a full scan (no groupable/filterable timestamp) |
| 10 | `compact_tombstones` | ❌ **fails, by design** | **No backend primitive** — `Memory` has no `deleted_at`, so tombstoned ids can't even be *identified*, let alone purged. `Err`, not `Ok(0)`, so an operator isn't told compaction succeeded when it couldn't run at all |
| 11 | `pull_memories` | ✅ pass, full-scan-and-sort | No `ORDER BY`, no range-filterable timestamp field — `SELECT * FROM memory` (whole table), sorted client-side by `(updated_at_unix_ms, id)`, then filtered/paged in Rust |
| 12 | `pull_entities` | ✅ pass, ordering degraded | Same full-scan approach; sorted by id only since there's no timestamp to sort by at all |
| 13 | `pull_links` | ✅ pass, ordering degraded | A `JOIN ... ON mentions` gives every `(memory_id, entity_id)` pair directly via `left_id`/`right_id` on the joined rows — genuinely cheap and correct — but edges carry no `created_at`, so the requested keyset cursor is approximated by sorting the pair itself |
| 14 | `pull_entity_relations` | ✅ pass (`[]` always) | No backend primitive — consistent with #3's `Err` on write; reads honestly return nothing rather than erroring, since "no relations exist" is true |

**10 real integration tests, all green, run against a live `persistent_server` instance**:
`ping_and_migrate_succeed`, `insert_then_lww_replace_then_lww_loss`,
`entity_insert_and_replace_preserves_mention_count`,
`link_is_insert_or_ignore_and_pull_links_reports_it`,
`entity_relation_has_no_backend_primitive` (asserts the `Err`),
`compact_tombstones_has_no_backend_primitive` (asserts the `Err`),
`approx_count_tables_is_always_none`, `count_tables_and_count_by_category_are_real`,
`count_by_origin_node_is_always_empty`, `stats_reports_real_totals_and_named_defaults`.

**Not done:** a full mechanical port of the existing ~60-test SQLite/Postgres suites. I
judged a focused 14-method coverage suite that honestly asserts every gap (rather than
silently skipping the cases those suites assume away) was the more useful use of the
spike's time; porting the full suites is flagged as follow-up work below.

### Missing wire/server capabilities (with the request that would close each gap)

1. **No compare-and-swap on `Replace`.** The wire's `Replace` is an unconditional
   overwrite. LWW sync needs `get` → compare → `replace`, three round trips and not atomic
   against a concurrent writer — unlike the reference's single `UPDATE ... WHERE
   excluded.updated_at > memories.updated_at`. **Request that would close it:** a
   conditional variant, e.g. `Request::ReplaceIf { id, fields, expect_field, expect_value }`
   answered `Ok`/`ConditionFailed`/`NotFound`.

2. **No `ORDER BY`, and no range-filterable/scannable timestamp field.** Of `Memory`'s
   eleven fields, only `category` is `filter_eq` and only `access_count` is `scan`-capable;
   `updated_at_unix_ms` is neither. Every keyset pull in this store is a full-table scan.
   **Request that would close it:** either make `updated_at_unix_ms` `scan`-capable (so
   `Aggregate`'s `MIN`/`MAX` and a range-filtered `ScanField` work) or add a monotonic
   sequence field the way the hub's own `hub_seq` already is.

3. **No directed, open-label entity-to-entity relation.** `Entity`'s two relation labels
   are fixed and symmetric (`relates_to`, `mentioned_with`); the hub's `entity_relations` is
   directed with an arbitrary `relation` string. Both are named non-goals in
   `rusty_multimodal_db`'s own `docs/design/SERVER-LINK-DESIGN.md` and `PROJECT-STATUS.md`
   item 148. **Request that would close it:** a `Request::LinkDirected { left, relation:
   String, right }` on an open-label, directed edge store — a materially different design
   than symmetric `Link`, not a small addition.

4. **No `deleted_at`/tombstone field on `Memory`.** `ADR-0048` named soft deletion a
   non-goal at design time; the real hard runtime deletion `ADR-0051` later added has no
   per-record "when was this tombstoned," so there's no way to answer "which records were
   tombstoned before cutoff X" at all. **Request that would close it:** either add a
   `deleted_at` field to the `Memory` projection, or (cleaner, given `ADR-0051` already
   exists) a `Request::ListDeletedSince { since }` the server itself can answer from its own
   tombstone log.

5. **No global edge-count primitive.** `count_tables("memory_entities")` costs one
   `neighbors_by_relation` round trip per memory — O(n), not O(1). **Request that would
   close it:** an `Aggregate`-style `CountEdges { relation }`.

### Anything that behaved differently from its docs

Nothing contradicted the docs during the actual run — the discrepancies found were between
docs and *other* docs/schema (the 28-vs-30 column count above), not between behavior and
documentation. The one implementation-time surprise: `rusty_multimodal_db`'s embedded SQL
subset requires the JOIN's selected columns to belong to either side by name (`m.content`,
`e.label`) but the `left_id`/`right_id` on a `Joined` row are populated regardless of which
columns are selected — meaning `pull_links` can select a single cheap column and still get
full edge identity, which isn't spelled out anywhere but is exactly the right behavior and
made that method far cheaper than expected.

### Id-mapping design choice, flagged as a decision this spike did not make for you

The consumer's memory ids (`mem_<uuid4-simple>`) round-trip losslessly into `Uuid`. Its
entity ids (`crates/remind_me_core/src/entity.rs::entity_id` — first 12 hex chars, 48 bits,
of a SHA-256 of the normalized name) do **not** — there's no lossless 48-bit-into-128-bit
mapping. This spike zero-pads the 12 hex chars into a `Uuid`'s low 48 bits and strips them
back out, which is invertible only for ids this store itself produced that way, and claims
a 128-bit uniqueness property the real 48-bit key space doesn't have.
`rusty_multimodal_db` already has its own collision-resistant `entity_id()` (full SHA-256
→ 128 bits, called out in that crate's own `sha2` dependency comment as deliberately *not*
inheriting this 48-bit truncation) — a real integration should probably use that instead
and accept that the id a synced client sees for an entity would differ from the consumer's
own id. That's a bigger, cross-repo decision, named here rather than made unilaterally.

---

## Bottom line, options as the task brief asked for

- **(a) Adopt as-is with the documented gaps** — usable today for `memories` sync only;
  `entity_relations` sync and tombstone compaction would need to stay on the current
  backend, or be added to the wire first (items 3–4 above).
- **(b) Fix the wire first** — items 1–2 above (conditional replace, a range-filterable
  timestamp) would remove the two biggest correctness/perf caveats; items 3–4 are
  larger, separate design rounds in `rusty_multimodal_db` itself.
- **(c) Close as evidence-gathering only** — this spike stands as the answer to "can it
  back the hub," and no further work happens here.

## What's left, and how to get it out of this sandbox

- Everything is committed on `claude/multimodal-hub-spike` (1 commit, `4f15d06`) in the
  cloned repo — but this container has no GitHub credentials, so the branch is local only.
- Attached: `0001-multimodal-hub-spike.patch` (git-format-patch of that commit) and this
  report. To get it onto GitHub from your own machine:
  ```
  git checkout -b claude/multimodal-hub-spike origin/main
  git am 0001-multimodal-hub-spike.patch
  git push -u origin claude/multimodal-hub-spike
  gh pr create --title "Spike: MultimodalHubStore" --body-file spike-report.md --draft
  ```
  (the task said open, don't merge — `--draft` or a plain `gh pr create` without merging
  both satisfy that)
- Not done, worth naming as follow-up: a full port of the existing SQLite/Postgres
  differential test suites; a decision on the id-mapping question above; and, if option
  (b) is picked, the two wire-change design rounds in `rusty_multimodal_db` itself.
