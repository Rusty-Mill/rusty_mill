# Overlap review — `nexus-memory` / `nexus-memory-hub` vs `remind_me_core` / `remind_me_hub`

Reviewed on 2026-09-23. **Dry run, report only** — no code, manifest, or
README changes accompany this document, and no PRs or issues were opened.

## Baseline and caveat

The `rusty_remind_me` subtree import is **not on `main` yet**. It exists only
on the unmerged branch `origin/claude/keen-tesla-c8qhx6`, tip
`e4ff1c27cc8decdee511e0687706b400e4f7b8a7` ("rusty_remind_me: release, plugin
marketplace and CI from the monorepo (ADR-0004)"). This review reads a clean
detached worktree at that commit. `crates/apps/nexus` is byte-identical
between that commit and `main` @ `6bfb142ea`, so the nexus line numbers below
also hold on `main`. The `rusty_remind_me` line numbers are valid at
`e4ff1c27c` and on `main` once that branch merges unchanged.

Paths are repo-root-relative. `RM` = `crates/apps/rusty_remind_me/crates`,
`NX` = `crates/apps/nexus/crates`.

Size, for scale (lines of `.rs`): `remind_me_core` ≈ 70.2k, `remind_me_hub` ≈
5.3k; `nexus-memory` ≈ 6.8k, `nexus-memory-hub` ≈ 0.7k.

## Verdict

**Diverged duplication (convergent-but-diverged), not exact or near code
duplication.** No source was copied between the two families: `grep -rni
nexus` over `crates/apps/rusty_remind_me` has zero hits, and nexus re-derived
the design independently (different id type, different tombstone encoding,
different connection model, different HTTP stack, different vitality formula).
What *is* duplicated is a **contract**: the `memories` row shape, the FTS5
table, and a subset of the hub's push/pull wire protocol. The two sides
agree on that contract only partially, and they disagree on semantics in ways
that cause silent data loss today if the two ever share a hub or a database
(see [Interop defects](#interop-defects-found-in-passing)).

The root README row calling `nexus-memory` "full `remind_me` schema/API
parity" is **overstated**. The nexus side covers the `memories` table
and roughly 22 IPC verbs. `remind_me_core` has 62 public modules, around 25
tables, a v29 migration reconciler, an entity graph, history, reminders, saved
searches, feedback, and in-process vectors.

**Decision (owner, 2026-09-23).** `nexus-memory` and `rusty_remind_me` are
different products and stay separate. Where they genuinely duplicate each
other, `rusty_remind_me` wins. The only component that duplicates, rather
than merely resembles, is the sync hub. So **`remind_me_hub` replaces
`nexus-memory-hub`**, and **no shared `crates/libs/` crate is extracted**.
See [Hub replacement plan](#hub-replacement-plan).

Why this fits the evidence:

- `nexus-memory` vs `remind_me_core` is not duplication. They differ in id
  type, tombstone, versioning, connection model, ranking, vectors and API
  surface (table below). Extracting shared code would mean choosing one
  product's design for the other.
- `nexus-memory-hub` describes itself as a mirror of the `remind_me` hub
  protocol (`NX/nexus-memory-hub/README.md:7`). `remind_me_hub` serves every
  route the nexus client calls, and more. That is duplication, and the
  original is the more complete implementation.
- The replacement is a runtime change, with nexus nodes pointing at a
  `remind_me_hub` URL. It adds no Cargo edge, so ADR-0003 is not involved.

The original analysis, which also considered extracting a wire-contract
crate, is kept below as evidence. That option is **superseded** by the
decision above.

Details, evidence, and blockers follow.

## Dimension-by-dimension comparison

### 1. Schema — near on the `memories` table, diverged overall

| Aspect | nexus-memory | remind_me_core | Class |
|---|---|---|---|
| `memories` columns | 23 columns, `NX/nexus-memory/src/db.rs:26-50` | 28 columns (v29), `RM/remind_me_core/src/db/schema_tables.sql:55-64` | **near**: nexus is a strict subset, missing `doc_id`, `chunk_index`, `deleted_at`, `remind_at`, `sensitive` |
| Column defaults | identical literal defaults (`'general'`, `'[]'`, `'{}'`, `'manual'`, `'unknown'`, `0.1`, `1.0`) | same | exact |
| `memories_fts` | `fts5(content, category, tags, content='memories', content_rowid='rowid')`, `NX/.../db.rs:60-63` | same definition, `RM/.../schema_tables.sql:66-70` | **exact** |
| FTS triggers | `memories_ai/_ad/_au`, `NX/.../db.rs:65-78` | same names and bodies, `RM/.../schema_triggers.sql:28-43` | exact |
| `chat_imports` | `NX/.../db.rs:52-58` | `RM/.../schema_tables.sql:12` | exact shape |
| Other tables | `sync_state` k/v, plus nexus-only `episodic_log`, `semantic_facts`, `procedural_skills` (`NX/.../db.rs:85-126`) | around 25 tables: `entities`, `entity_relations`, `memory_entities`, `memory_revisions`, `memory_tags`, `sync_outbox`/`sync_log`/`sync_sends`/`sync_flags`, `vec_chunks`/`vec_embeddings`, `wiki_*`, `saved_searches`, `reminder_deliveries`, `memory_feedback`, … (`RM/.../schema_tables.sql:4-195`; `RM/remind_me_core/src/vectors.rs:38-41`) | diverged |
| Outbox and tag triggers | none | trigger-fed `sync_outbox` gated on `sync_flags`, plus the `memory_tags` index, `RM/.../schema_triggers.sql:4-91` | diverged |
| Versioning | **none**: `CREATE … IF NOT EXISTS` on every open (`NX/.../db.rs:21-25, 208`) | `PRAGMA user_version`, `SCHEMA_VERSION = 29` (`RM/remind_me_core/src/db/migrations.rs:49`), a reconciler that rebuilds tables whose DDL differs (`migrations.rs:478-527`) | **diverged** |
| Schema source of truth | hand-written in Rust | generated verbatim from the Python reference's `sqlite_master`, marked "do not hand-edit" (`RM/.../schema_tables.sql:1-2`) | diverged |

### 2. Model — diverged

| Field | nexus `Memory` (`NX/nexus-memory/src/model.rs:109-158`) | remind_me `Memory` (`RM/remind_me_core/src/models.rs:59-116`) |
|---|---|---|
| `id` | `Uuid`, v7 for new rows (`model.rs:113, 167`) | `String`, `"mem_" + uuid v4 simple` (`RM/remind_me_core/src/db/queries.rs:101`) |
| timestamps | `DateTime<Utc>` | `String` (RFC 3339) |
| `memory_type` | closed enum `Episodic/Semantic/Procedural/Unclassified` (`model.rs:23-33`) | free string; values such as `decision`, `preference`, `fact` drive decay (`RM/remind_me_core/src/vitality.rs:21-36`) |
| `status` | closed enum incl. **`Deleted`** as the tombstone (`model.rs:64-79`) | free string (`active`/`dormant`); the tombstone is the separate **`deleted_at`** column |
| extra fields | none | `doc_id`, `chunk_index`, `remind_at`, `sensitive`, `deleted_at` |

The same column names carry **different vocabularies**. `memory_type =
"semantic"` means something to nexus and nothing to remind_me's decay table,
and the reverse holds for `"decision"`.

### 3. Storage — diverged

| Aspect | nexus-memory | remind_me_core |
|---|---|---|
| Connection | `r2d2` pool (`NX/nexus-memory/Cargo.toml` deps; `NX/.../db.rs:15`) | single `parking_lot::Mutex<Connection>` plus an optional secondary connection for the sync worker (`RM/remind_me_core/src/db/mod.rs:166-231`) |
| Pragmas | WAL, `busy_timeout=5000` (`NX/.../db.rs:175`) | WAL, `busy_timeout=30000`, `foreign_keys=ON` (`RM/remind_me_core/src/db/schema.rs:13-27`) |
| UDFs | none | `effective_vitality(...)` scalar function registered per connection (`RM/remind_me_core/src/vitality.rs:104-128`) |
| `rusqlite` | `{ workspace = true }` → `0.39` + `bundled`,`backup` (root `Cargo.toml:476`) | literal `0.39` + `bundled`,`backup`,**`functions`** (`RM/remind_me_core/Cargo.toml:21`; `RM/remind_me_hub/Cargo.toml:15`) |
| DB file | `<forge>/.forge/memory/memory.db`, shared with nexus's cognitive tables (`NX/nexus-memory/src/lib.rs:103-116`) | `REMIND_ME_DB_PATH` / `~/.remind-me/memory.db` (`RM/remind_me_core/src/db/mod.rs:10-22`) |

### 4. Search and ranking — diverged, with a shared RRF constant

| Aspect | nexus-memory | remind_me_core |
|---|---|---|
| FTS query shaping | whole input wrapped as **one literal phrase** (`NX/.../db.rs:639-643`) | tokenized, each token quoted, **joined with `OR`** (`RM/remind_me_core/src/fts.rs:19-29`) |
| FTS filters | `status != 'deleted'`, `ORDER BY rank` (`NX/.../db.rs:633-650`) | `superseded_by IS NULL AND deleted_at IS NULL`, vitality floor 0.05, `category`, `sensitive = 0`, `bm25()` (`RM/remind_me_core/src/db/queries.rs:828-857`) |
| Embeddings | delegated over kernel IPC to `com.nexus.ai::embed_text`, no local model (`NX/nexus-memory/src/vector.rs:1-17, 80-95`) | in-process `Embedder` trait with Ollama or ONNX backends, 384-dim default (`RM/remind_me_core/src/embedder.rs:54, 133, 722`) |
| Vector storage | delegated to `com.nexus.storage` `vector_insert/query`, namespace `memory` (`NX/.../vector.rs:30-35`) | local `vec_embeddings` BLOBs, brute-force dot product with optional `usearch` ANN (`RM/remind_me_core/src/vectors.rs:38-41, 295`) |
| Fusion | plain RRF, `k = 60`, two arms (FTS and vector) (`NX/.../vector.rs:39, 64`) | weighted RRF, `k = 60`, five signals (keyword, semantic, recency, vitality, idf), with query-shape weight routing (`RM/remind_me_core/src/retrieval.rs:25, 55, 266, 363`) |
| Post-rank | none | feedback adjustment, optional cross-encoder rerank, token-budget trim (`RM/.../queries.rs:960-996`) |
| Vitality | `base_weight·(1+n)/(1+decay·days)` (`NX/.../db.rs:1043-1050`) | ACT-R `base_weight·√(n+1)·e^(−decay·days)`, with decay halved once n ≥ 10 (`RM/remind_me_core/src/vitality.rs:65-91`) |

The same query ranks differently on the two sides by design. Nothing here is
hoistable without first choosing one ranking model as the product behaviour.

### 5. Sync protocol — near on the wire, diverged in semantics

**Wire surface both clients use** (compatible):

- `POST {hub}/sync/push`, body `{"node_id", "records":[…]}`, bearer auth
  (nexus `NX/nexus-memory/src/sync.rs:272-277`; remind_me
  `RM/remind_me_core/src/sync/push.rs:131, 147`)
- `GET {hub}/sync/pull?since=&since_id=&exclude_node=`, keyset on
  `(updated_at, id)`, reply `{records, count}` (nexus `sync.rs:310-319`;
  remind_me `RM/remind_me_core/src/sync/pull.rs:213-316`)
- push page of 200 on both sides (nexus `sync.rs:42`; remind_me `push.rs:16-18`)
- last-write-wins on a strictly greater `updated_at` string

**Where they diverge:**

| Aspect | nexus-memory client | remind_me_core client |
|---|---|---|
| Change capture | scans `memories` by keyset cursor; **pushes only rows it authored** (`node_id` unset or its own), so edits to foreign rows never propagate (`NX/.../sync.rs:237-266`, limitation stated at `sync.rs:15-19`) | trigger-fed `sync_outbox`, authorship-agnostic, per-remote `sync_sends` (`RM/.../schema_triggers.sql:45-60`; `schema_tables.sql:155-169`) |
| Tombstone | `status = 'deleted'` (`NX/.../db.rs:740-747`) | `deleted_at = now` (`RM/remind_me_core/src/db/queries.rs:428-433`) |
| Merge on apply | whole-row replace (`NX/.../db.rs:261-278`) | LWW plus **tag union and metadata shallow merge** (`RM/remind_me_core/src/sync/record.rs:199-230, 256-292`) |
| Record types | memories only | `record_type` dispatch: memory, entity, memory_entity and entity_relation (`RM/remind_me_core/src/sync/graph.rs:289-322`) |
| Cursor modes | keyset only | keyset, **or** hub `since_seq` / `hub_seq`, with a capability probe (`RM/.../pull.rs:26-38, 114-173`) |
| Topology | hub only | hub plus peer-to-peer (Tailscale discovery, `RM/remind_me_core/src/sync/peers.rs:187-208`) |
| Transport | async `reqwest` with TLS, an SSRF guard and DNS pinning (`NX/.../sync.rs:99-200`) | hand-rolled blocking `http://` over `TcpStream`, TLS via reverse proxy (`RM/remind_me_core/src/sync/http.rs:1-8, 33-36`) |

### 6. Hub — near on the wire (nexus is a subset), diverged in implementation

| Aspect | nexus-memory-hub | remind_me_hub |
|---|---|---|
| Stack | `axum` 0.8 + `tokio` (`NX/nexus-memory-hub/Cargo.toml`) | hand-rolled HTTP/1.1 on `std::net`, a thread per connection (`RM/remind_me_hub/src/main.rs:127-148`; `src/http.rs:1-13`) |
| Routes | `/health`, `/sync/push`, `/sync/pull` (`NX/nexus-memory-hub/src/lib.rs:299-303`) | the same three plus `/stats`, `/count`, `/metrics`, `/admin/compact_tombstones`, `/sync/pull_entities`, `/sync/pull_links`, `/sync/pull_entity_relations` (`RM/remind_me_hub/src/lib.rs:148-183`) |
| Storage | **schema-agnostic**: one `records(id, updated_at, node_id, origin_node, payload JSON)` table (`NX/.../lib.rs:67-76`) | **typed** full columns plus `hub_seq`, entity-graph tables, SQLite *or* Postgres behind a `HubStore` trait (`RM/remind_me_hub/src/store/sqlite.rs:38-105`; `store/mod.rs:152-194`) |
| LWW guard | `WHERE excluded.updated_at > records.updated_at` (`NX/.../lib.rs:155`) | `WHERE excluded.updated_at > memories.updated_at` (`RM/remind_me_hub/src/store/sqlite.rs:363`) |
| `exclude_node` | filters `origin_node` (`NX/.../lib.rs:207-213`) | filters `origin_node`, with `full=1` to disable (`RM/remind_me_hub/src/store/sqlite.rs:639-642`) |
| Validation | needs only `id` and `updated_at`; rejects timestamps more than 5 minutes in the future (`NX/.../lib.rs:51, 158-166`) | needs `id`, `content`, `created_at`, `updated_at` (`RM/remind_me_hub/src/record.rs:217-221`); canonicalizes timestamps (`RM/remind_me_hub/src/canon.rs:27-53`) |
| Limits | default pull 100, max 500 (`NX/.../lib.rs:44-46`); no push-batch cap found | default and max pull 500, push cap 1000 (`RM/remind_me_hub/src/store/mod.rs:66, 73`) |
| Auth | `SYNC_SECRET` bearer, constant-time (`NX/nexus-memory-hub/src/main.rs:30-47`; `lib.rs:317-335`) | `SYNC_SECRET` bearer, constant-time (`RM/remind_me_hub/src/lib.rs:83, 111-133`) — **exact** |
| Versioning | none | `HUB_VERSION = "1.6.0"`, diagnostic only; capabilities probed via 404 (`RM/remind_me_hub/src/lib.rs:58-64`) |
| Deploy | Containerfile plus Quadlet and systemd units (`NX/nexus-memory-hub/deploy/`) | its own release train (ADR-0004 on the import branch) |

Its own README claims nexus-memory-hub "mirrors the proven `remind_me` hub
wire protocol" (`NX/nexus-memory-hub/README.md:7`, `src/lib.rs:5-6`), and on
the three routes it implements, that claim holds.

### 7. Public API — diverged

- **nexus-memory:** a `MemoryDb` struct with methods (`NX/nexus-memory/src/db.rs:184-1012`), wrapped by the
  `com.nexus.memory` kernel-IPC plugin with 22 named handlers (`NX/nexus-memory/src/core_plugin.rs:58, 109-130`). It also exposes the
  in-memory `MemoryStore` cognitive facade (`NX/nexus-memory/src/lib.rs:75-83`). The plugin depends on `nexus-kernel`,
  `nexus-plugin-api` and `nexus-plugins` (`NX/nexus-memory/Cargo.toml`).
- **remind_me_core:** no facade type. `Database` only hands out a locked
  `&Connection` (`RM/remind_me_core/src/db/mod.rs:166-194`), and every operation is a free function over
  `&Connection`: `add_memory` `queries.rs:98`, `search_memories` `:723`, `update_memory` `:273`, `delete_memory` `:397`, entity, history,
  capture, reminder, wiki and sync functions (`RM/remind_me_core/src/lib.rs:1-67`). The
  `remind_me_mcp` crate exposes it as MCP tools.
- The verbs overlap by name (add/get/list/search/update/delete/stats/facts/
  entities/export/tags/vitality_report/auto_capture/get_capture/consolidate/
  wiki_*/sync). Signatures, id types and error types all differ.

## Classification summary

| Dimension | Class | One-line reason |
|---|---|---|
| `memories` + FTS5 DDL | **near** | nexus is a column subset of v29; the FTS table and triggers are identical |
| Rest of schema | diverged | nexus has 4 tables of its own; remind_me has around 20 more |
| Model / id / tombstone | diverged | `Uuid` vs `"mem_…"`; `status='deleted'` vs `deleted_at` |
| Storage / connection | diverged | r2d2 pool vs a mutexed single connection; different pragmas and UDFs |
| Search / ranking | diverged | phrase vs OR-token FTS; IPC vectors vs in-process; different vitality math |
| Sync wire (common subset) | **near** | same routes, envelope, cursor and LWW rule |
| Sync semantics | diverged | authorship scan vs outbox; merge rules; record types |
| Hub implementation | diverged | axum and opaque JSON vs std::net, typed rows and Postgres |
| Hub auth | exact | same env var, same bearer check |
| Public API | diverged | plugin IPC over a struct vs free functions plus MCP |

No dimension is **exact duplication of code**.

## Extraction recommendation (superseded, kept as evidence)

> Superseded by the 2026-09-23 decision in [Verdict](#verdict): there is no
> `libs/` extraction, and `remind_me_hub` replaces `nexus-memory-hub`.

### What not to extract

- **Storage or DB layer.** The two sides differ on connection model, pragma
  values, UDFs, the tombstone column, the id type, and whether a migration
  reconciler exists at all. A shared crate would force one side's choices on the other. If
  remind_me's reconciler (`migrations.rs:478-527`, which rebuilds tables whose DDL
  differs) were pointed at a nexus `memory.db`, it would restructure a file
  that also holds nexus's cognitive tables.
- **Search and ranking.** This is product behaviour, not plumbing, and the
  two products rank differently. Hoisting it means picking a winner first.
- **Vectors.** nexus delegates vectors to other nexus plugins over kernel IPC
  (`vector.rs:1-17`). That is a design decision ("D-1") tied to
  `nexus-kernel`, which cannot live in `libs/`.

### What to extract, once the blockers below are cleared

A single small crate, suggested name `crates/libs/protocol/memory_sync_wire`
(`libs/protocol/` already holds `rusty_mcp`, `rusty_a2a`, `rusty_acp` and
`rusty_lsp`, per ADR-0003 §Decision). It would hold:

- the serde record type for a synced memory (`id: String`), plus the
  `record_type` tag and the entity, link and relation variants;
- push and pull envelope types, and the keyset and `since_seq` cursor types;
- `canonical_timestamp()` (today `RM/remind_me_hub/src/canon.rs:27-53`) and
  the LWW comparison;
- the tombstone rule (one field, one meaning);
- a **hub conformance test suite** as a pub test-support module that both hubs
  (`remind_me_hub`, `nexus-memory-hub`) and both clients run.

Dependencies: `serde`, `serde_json` and `chrono` only. No `rusqlite`, no HTTP
stack, no async. That satisfies ADR-0003 (`libs` may depend on `foundation`,
`platform` and `libs`) and gives both apps the same edge direction. It also
keeps the `rusqlite` feature question out of the shared crate.

Optionally, and later: a `memories` + FTS5 DDL constant, but only after the
schema versioning question is settled (blocker 1).

### Alternative that avoids a crate

Keep the two apps separate and **retire `nexus-memory-hub`**, pointing nexus
nodes at a `remind_me_hub` deployment. That is a runtime dependency, not a
Cargo edge, so ADR-0003 is not involved. `remind_me_hub` already serves every
route nexus calls, and nexus's own README says it mirrors that hub. This
still needs blockers 2–4 fixed on the nexus client, because the defects below
would then occur on every sync.

## Hub replacement plan

The target is to delete `NX/nexus-memory-hub` and have nexus nodes sync
through a `remind_me_hub` deployment.

**Works today, for a fleet of nexus nodes only.** Everything nexus pushes passes
`remind_me_hub` validation. The `id`, `content`, `created_at` and `updated_at`
fields are all present in nexus's serialized `Memory` (`RM/remind_me_hub/src/record.rs:217-221`;
`NX/nexus-memory/src/model.rs:109-158`). Every nexus field has a matching hub
column. The nexus client's query parameters (`since`, `since_id`,
`exclude_node`) have the same meaning on the remind_me hub, including
`exclude_node` filtering on `origin_node`. The hub's default page of 500 is
at least nexus's `BATCH` of 200, so paging still terminates correctly.

**Must be fixed before nexus and remind_me nodes share one hub.** These are
interop defects 2–4 below:
1. The nexus pull path must accept `String` ids, or skip foreign ids visibly
   instead of dropping them silently (`NX/nexus-memory/src/sync.rs:335-347`).
2. Tombstones must agree. The recommended choice is for nexus to adopt
   `deleted_at` and keep `status` for lifecycle only.
3. Nexus must carry `sensitive`, `deleted_at` and `remind_at` through, so they
   are not dropped on pull.

**Must be done to delete the crate:**
1. **Test harness.** `nexus-memory` dev-depends on `nexus-memory-hub` for
   `tests/sync_hub.rs` (`NX/nexus-memory/Cargo.toml` `[dev-dependencies]`).
   Retargeting that dev-dependency at `remind_me_hub` would be an app-to-app
   edge. The layer checker counts dev and build edges
   (`.github/scripts/check_workspace_layers.py:5, 28`), so that is not allowed.
   Instead, spawn the built `remind-me-hub` binary as a subprocess, or replace
   the test with an in-crate mock of the three routes.
2. **Deploy files.** Replace `NX/nexus-memory-hub/Containerfile` and
   `deploy/*.container|*.service|hub.env.example` with a pointer to the
   `remind_me_hub` deploy files. The env var names differ: `SYNC_SECRET` is the
   same, but bind and DB path become `REMIND_ME_HUB_BIND`, `REMIND_ME_HUB_PORT`
   and `REMIND_ME_HUB_DB_PATH`, or `DATABASE_URL`.
3. **Data migration.** Existing `nexus-memory-hub` databases (`records.payload`
   JSON) need a one-shot export, then a re-push. Alternatively, clear the hub
   and have every node re-push from the epoch by resetting `sync.push.*` in
   `sync_state`.
4. **Docs.** Update the root `README.md:276` row, the nexus crate list
   (`crates/apps/nexus/docs/0.1.2/crates.md`), and the nexus CHANGELOG.
5. **Verify one behaviour.** `remind_me_hub` rewrites `updated_at` to its
   canonical form (`RM/remind_me_hub/src/canon.rs:27-53`), while nexus writes
   `to_rfc3339()`. Test that the equal-timestamp string comparison in nexus's
   `upsert_lww` still behaves correctly after a round-trip. This was not run
   during the review.

**Precondition:** merge `origin/claude/keen-tesla-c8qhx6` first.

## Blockers to extraction (or to sharing a hub)

1. **Schema versioning mismatch.** nexus has no `user_version` and applies
   `IF NOT EXISTS` DDL (`NX/nexus-memory/src/db.rs:21-25`). remind_me is at
   `SCHEMA_VERSION = 29` with a reconciler (`RM/remind_me_core/src/db/migrations.rs:49, 478-527`). nexus lacks five v29
   columns. The v29 DDL is generated from the Python reference and marked
   "do not hand-edit" (`schema_tables.sql:1-2`), so a shared DDL would have to be
   generated too, or remind_me would give up that provenance.
2. **Id type.** nexus `Uuid` (`model.rs:113`) vs remind_me `"mem_" + hex`
   (`queries.rs:101`). The wire type has to be `String`, and nexus's
   `Memory.id: Uuid` would have to change. That touches `MemoryDb`,
   `vector.rs:49-54` (vector paths keyed by `Uuid`) and every IPC arg type
   exported via `ts-export`.
3. **Tombstone encoding.** `status = 'deleted'` (`NX/.../db.rs:740-747`) vs
   `deleted_at` (`RM/.../queries.rs:428-433`). Neither side recognizes the
   other's form.
4. **Merge semantics.** Whole-row LWW (nexus) vs LWW plus tag union and metadata merge
   (remind_me node, `record.rs:199-230`). Both hubs do whole-row replacement. A shared
   crate has to pick one rule, or expose both as policy (mechanism/policy split).
5. **Sync wire scope.** nexus has no `record_type`, no graph endpoints, no
   `hub_seq`/`since_seq`, no push-batch cap and no timestamp canonicalization.
   A shared crate either carries the full remind_me surface, which leaves the
   nexus hub non-conformant until it grows, or feature-gates it.
6. **`rusqlite` features.** remind_me declares `functions` literally
   (`RM/remind_me_core/Cargo.toml:21`, `RM/remind_me_hub/Cargo.toml:15`), while the
   workspace entry is `bundled`,`backup` (root `Cargo.toml:476`). Because
   `rusqlite`'s `links = "sqlite3"` allows exactly one version in the graph
   (root `Cargo.toml:471-475`), versions are already forced to `0.39`, and
   feature unification means a whole-workspace build already compiles
   `functions` for nexus too. This is a **hygiene item, not a hard blocker**:
   move remind_me to `workspace = true` and add `functions` to the root
   entry. It is a blocker only for a `rusqlite`-bearing shared crate, which the
   recommendation above avoids.
7. **Transport and dependency philosophy.** nexus uses `axum`, `tokio` and
   `reqwest`+TLS; remind_me deliberately hand-rolls `std::net` HTTP with no
   TLS (`RM/remind_me_core/src/sync/http.rs:1-8`), in the spirit of ADR-0002
   sovereignty. A shared crate must therefore be transport-free, which is why the
   recommendation stops at types and a conformance suite.
8. **Postgres.** `remind_me_hub` defaults to the `postgres-store` feature
   (`RM/remind_me_hub/Cargo.toml`), and the nexus hub is SQLite-only. That is irrelevant to a
   wire crate and a blocker to any shared hub *store*.
9. **Independent release trains.** `rusty_remind_me` ships its own releases
   and plugin marketplace from the monorepo (ADR-0004, import-branch commit
   `e4ff1c27c`). A shared `libs/` crate couples two release cadences, so it
   should be versioned conservatively and kept small.
10. **Import not merged.** Nothing can depend on `remind_me_*` layout on
   `main` until `origin/claude/keen-tesla-c8qhx6` lands.

## Interop defects found in passing

These are observations; nothing was changed. Each is confirmed by reading the
code but **not executed**.

1. **The nexus remind_me importer skips every real remind_me row.**
   `import_remind_me_db` calls `parse_uuid` on `id`
   (`NX/nexus-memory/src/import/remind_me_db.rs:105`). remind_me ids are
   `"mem_<32 hex>"` (`RM/remind_me_core/src/db/queries.rs:101`), which
   `Uuid::parse_str` rejects. The error is swallowed into `report.skipped += 1`
   (`remind_me_db.rs:73-79`). The tests build their source rows with
   `Uuid::now_v7()` ids (`remind_me_db.rs:179, 194`), so they pass. **Severity:
   high for the "1:1 import" claim** (`NX/nexus-memory/src/model.rs:3-5`).
2. **A nexus client pulling from a hub that holds remind_me rows drops them
   silently.** Pull decodes each record as `Memory` (id `Uuid`), ignores decode
   failures, and still advances the cursor past them
   (`NX/nexus-memory/src/sync.rs:335-347`). Those rows are never retried.
3. **Deletes do not cross the boundary in either direction.** A nexus tombstone arrives at a
   remind_me node as `status: "deleted"`, which remind_me does not filter on,
   so the memory stays visible there. A remind_me tombstone (`deleted_at`) is
   an unknown field to nexus's `Memory` and is dropped by serde, so the row
   stays live.
4. **The `sensitive` flag is dropped** when a remind_me record enters nexus,
   because `Memory` has no such field. Sensitive memories would surface in nexus
   search. This is privacy-relevant if hubs are ever shared.
5. *(remind_me-internal, inferred.)* The `delete_memory` doc comment
   (`RM/remind_me_core/src/db/queries.rs:380-389`) says tombstones don't
   propagate because the outbox trigger lacks `deleted_at`. The trigger does
   carry it (`RM/remind_me_core/src/db/schema_triggers.sql:50, 59`), so the comment
   is stale.
6. *(remind_me-internal, inferred.)* The peer server's `/count` never
   returns `by_category` (`RM/remind_me_core/src/sync/server.rs:321-335`).
   Peer reconcile compares per-category counts (`RM/remind_me_core/src/sync/reconcile.rs:140-156`),
   so it likely classifies every non-empty peer as `NodeAhead`. The hub path
   was fixed for exactly this issue.
7. **README accuracy.** The root `README.md:275` "full `remind_me`
   schema/API parity" should be softened to something like "`remind_me`-compatible
   `memories` schema subset and hub wire subset" (a docs fix for later).

## Suggested sequencing (not actioned)

1. Merge the `rusty_remind_me` import branch.
2. Fix defects 1–4 in nexus-memory: `id: String`, adopt `deleted_at` as the
   tombstone, carry `sensitive`/`remind_at`/`deleted_at`, and surface decode
   failures instead of skipping them. This is valuable whether or not anything is
   extracted.
3. Replace `nexus-memory-hub` with `remind_me_hub`, following the
   [Hub replacement plan](#hub-replacement-plan). This was decided on 2026-09-23.
4. No `libs/` extraction. `nexus-memory` and `remind_me_core` stay separate
   products.

## Method

- A detached worktree at `e4ff1c27c`. Both nexus crates were read directly
  (`lib.rs`, `model.rs`, `db.rs`, `sync.rs`, `vector.rs`,
  `import/remind_me_db.rs`, `core_plugin.rs` handler table, and the hub's
  `lib.rs` and `main.rs`).
- `remind_me_core`, `remind_me_hub` and `remind_me_remote` were mapped by two
  read-only exploration passes, and the load-bearing citations were then spot-checked by hand:
  `SCHEMA_VERSION`, the `memories` DDL, the `rusqlite` feature lines, the hub LWW
  guard, required keys, the `delete_memory` tombstone path, `sanitize_fts_query`,
  `calculate_vitality`, and the id format.
- `remind_me_remote` turned out to be an MCP Streamable-HTTP connector
  (`RM/remind_me_remote/src/lib.rs:1-10`), not a sync client. It has no nexus
  counterpart and is out of scope for the overlap.
- Nothing was built or run. Every defect above is a code-reading
  finding.
