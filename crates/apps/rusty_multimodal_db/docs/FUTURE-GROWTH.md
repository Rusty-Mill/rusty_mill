# Future Growth

This document captures directions this project could grow in beyond its current scope, and what each would actually require. Nothing here is planned or scheduled — it exists so a future decision to pursue any of it starts from an honest accounting instead of a guess.

## Nothing here is architecturally blocked

Every current boundary in this project is a deliberate scope line from a specific round, not a structural limitation:

* The multi-process append fix targets local filesystems only (`O_APPEND`'s atomicity guarantee excludes NFS) — a real, identifiable piece of work if that assumption ever needs to change, not a rewrite.
* The multi-process fix covers slot creation specifically. Broader multi-writer coordination wasn't needed yet, not ruled out.
* The `research` feature flag means every benchmarked alternative — 4 storage backends, 8 durability variants, 4 concurrency strategies — is still in the codebase, just not compiled into a default build. Nothing was deleted at any point in this project's history.
* The generic schema layer (`crate::generic`) was validated against a toy domain (`Order`/`Customer`) and a real one (requirements traceability). Nothing schema-specific is baked into the storage engine itself.
* Staying off crates.io is a current decision (a `Cargo.toml`/publishing choice), not a technical constraint.

## Path to a server / query layer — since built

**Status (2026-09):** this path was taken. The `server` feature (`ADR-0010`, `SERVER-001`) is the binary described below, and every "genuinely new" item was built in its own round: authentication/authorization (`ADR-0012`), transport encryption (`ADR-0014`, mutual TLS `ADR-0023`), transaction sessions (`ADR-0024`, journal `ADR-0025`/`0026`, read-your-writes `ADR-0027`, snapshot isolation `ADR-0033`), a versioned protocol (`ADR-0022`), a client-side SQL subset (`ADR-0034`/`0035`/`0044`), and — for the owner's `rusty_remind_me` service — runtime insertion, linking, replacement, deletion, tables on one connection, and compaction (`ADR-0046`–`ADR-0052`), then — after an integration spike measured the remaining gaps — a durable data directory, an atomic guarded replace, ordered keyset pages, the consumer's sync fields, a global edge count, and its directed open-label edges as a record table (`ADR-0053`–`ADR-0058`). Every gap the spike named is closed. The original accounting is kept below as written, because it was right about the shape: the engine did not change to accommodate any of it.

The storage engine's public API (`get`/`scan`/`filter`/`update`/relationship traversal) is already a clean boundary a network layer could sit on top of without changing the engine itself.

Genuinely additive — no rework of the storage layer required:

* A binary that owns the store and listens on a socket, translating requests into calls against the existing API and serializing results back out.
* Concurrency across client connections is actually simpler than the cross-process case already solved: if one server process owns the file, client requests never touch it directly — this collapses back to the already-solved in-process concurrency problem (`RwLock`), not the harder cross-process one.
* A query language would compile down to primitives that already exist (filter-by-field, scan, relationship traversal) — real design work, but the storage layer wouldn't need to change to accommodate it.

Genuinely new — not incremental extensions of existing work:

* Authentication/authorization. Doesn't exist in any form today; a network-exposed store needs it from the start, not as an add-on.
* Session/transaction semantics across multiple requests. Every operation today is single-shot; a protocol has to define what a "connection" guarantees across several of them.
* The query language itself — real parser and language design, not a small extension.

## Path to SQLite/DuckDB parity

This is a different tier of project, not a natural extension of the current one — each of the three items below is roughly a multi-year effort on its own, and SQLite and DuckDB aren't even the same target to aim at (SQLite: row-store, transactional; DuckDB: column-store, analytical). "Parity with both" isn't one destination.

The big three:

1. SQL. A parser, a query planner, a cost-based optimizer, and an execution engine. Every query today is a hand-written Rust method call against a specific schema's traits — there's no declarative language layer at all. *Partly built since this was written:* a client-side `SELECT`/`GROUP BY`/`JOIN`/`ORDER BY` subset (`ADR-0034`/`0035`/`0044`/`0061`) compiled to fixed request shapes, and one ordered keyset page on the wire (`ADR-0055`) — `ORDER BY` in the SQL text (`ADR-0061`) compiles to it directly, for a plain query, and — since `ADR-0068`, protocol 26 — `ORDER BY` combined with `WHERE` too, compiling to `Request::FilteredPage` (`Page`'s own three fields plus a filter, evaluated by one shared default: `scan_all` → filter via the existing predicate evaluator → `Page`'s own key selection; a filtered request against `Memory`/`Relation` loses the `Ordered` index's speed advantage, named plainly, not hidden — the unfiltered fast path is untouched). *Since `ADR-0073`:* the first planner step — a `Request::Query` whose `WHERE` carries an equality on a field the domain declares `filter_eq: true` reads that index's bucket (`filter_eq` + per-id `get`) instead of `scan_all`, with every predicate re-checked over the fetched rows so the result set is the full scan's by construction; schema-driven and value-blind, no cost model. *Since `ADR-0074`:* the same candidate step behind `Aggregate`, the `FilteredPage` default (zero observable difference — its output order is a function of the filtered set), and `Join`'s left side, through one shared helper — every filtered read shape the server offers now narrows through a declared equality index. *Since `ADR-0075`:* the range path — a `WHERE` carrying `<`/`<=`/`>`/`>=`/`=` on the one field an adapter reports range-indexed (`Memory`/`Relation`'s `updated_at_unix_ms`, `ADR-0059`'s `Ordered` index) walks the `BTreeSet<(key, id)>` between the bounds instead of `scan_all`, through the same shared candidate step, so `Query`/`Aggregate`/`FilteredPage`/`Join` all narrow on it; equality-first, so no request that planned the equality index changes plan; declared server-side (`ConnectionStore::range_field`/`range_ids`), deliberately not a wire-visible `FieldCapabilities` flag. Still absent: any cost-based optimizer or selectivity estimate (the planner never *declines* an index it could use, never intersects an equality with a range; since `ADR-0083` two bounds on one side of the range field walk between the tightest two — a comparison between literals, not an estimate), any cost model beyond one measured ratio (since `ADR-0078` a filter carrying both an equality on the declared index and a bound on the range field reads only the ids in *both* lists — exact, no estimate — and since `ADR-0079` the range's id walk is abandoned past a budget of ten ids per bucket id, the crate's first and only cost constant, so the worst case is bounded at ~1.4× the bucket alone rather than the range's whole id list; the budget is a constant, not a setting, and estimates nothing; since `ADR-0077` a page ordered by the range field walks past any other predicate's rejects until it fills, and the pure-bounds `since`-shaped page costs the page since `ADR-0076`), a sorted index on any field but `Memory`/`Relation`'s `updated_at_unix_ms` and, since `ADR-0080`, `Reminder`'s `due_at_unix_ms` (each other domain is "a wrap and a `range_field` change, when a consumer asks"), `WHERE id = …` (not expressible in today's `Predicate`), and `ORDER BY` combined with `JOIN`/`GROUP BY` (`Page`/`FilteredPage`'s own Non-goal, unchanged — grouped/joined output is not the raw per-record field either one walks).
2. Transactions. *Partly built since this was written:* a batch of field updates applies all-or-nothing (`ADR-0013`), a per-connection session stages writes and commits them as one batch with read-your-writes and snapshot isolation (`ADR-0024`/`0027`/`0033`), an opt-in redo journal makes a `Request::Transaction` batch crash-atomic (`ADR-0025`), and — since `ADR-0063` — the same is now true of `Request::WriteBatch`'s atomic mode on `Memory`/`Entity`/`Relation` (journal format version 2, when the adapter is journaled). *Now built, since `ADR-0072`:* real MVCC — `SESSION_MVCC_ISOLATION` (protocol 27) wires the phase-1 spike's proven mechanism (`ADR-0071`, `MVCC-SPIKE`) into the server for real, for `Memory`/`Entity`/`Relation`: activation with a one-time baseline scan, snapshot-consistent `GetById` that survives *any* concurrent write (a session commit or an ordinary single-shot/atomic-batch write, not only another session), write-write conflict detection at `Commit`, and `Compact`-triggered flush-and-reclaim, verified end-to-end including across a real process restart. *Since PRs #235–#241 (2026-09-18):* all six domains are wired (`Dog`/`Order`/`Employee` too), the Rust and Python clients can open an MVCC session, the insert-log restart-recovery gap is closed by a per-domain `open_with_mvcc` (a pre-open read, no generic-layer change — `docs/design/MVCC-OPEN-HOOK-PROPOSAL.md`), every non-journaled write flushes its history durably (a real bug the deployment test found — history was only ever flushed at `Compact`/checkpoint before), and `memory_server` exposes it behind `SERVER_MVCC_ISOLATION`. Still absent, named explicitly rather than glossed over: reconstruction from a pending *journal* remainder (the insert-log twin is closed; this one is `MVCC-OPEN-HOOK-PROPOSAL.md`'s own open question); MVCC and the crash-atomic journal together on one table (`memory_server` refuses the pair at startup rather than pretend they compose); independent review of the whole line; and a general transaction manager beyond this — multi-table atomicity and staged (not single-shot) transactions over inserts, links, replacements, and deletes — remains absent.
3. Arbitrary joins. *Partly built since this was written:* a `JOIN … ON <relation>` over a declared relation, within a table or across two tables on one connection (`ADR-0044`/`ADR-0050`), evaluated as an index nested loop. Still absent: a join on an arbitrary predicate at query time — every relation is still declared ahead of time, and relation labels created at runtime (`ADR-0047`) are labels on a declared relation kind, not new predicates.

Smaller, but still real:

* Dynamic/runtime schema (`ALTER TABLE`-style changes). Schema here is a compile-time Rust concept, and a record layout is unversioned: adding a field is a schema-tag bump that refuses old directories distinctly (`ADR-0056`), with an in-place upgrade named as the round for the first directory that cannot be re-pushed.
* Null. The wire has no null; two domains carry documented per-field sentinels instead (`ADR-0056`: `0` for a timestamp, `""` for a node id — lossless for those columns). A nullable `ScanValue` at a protocol bump is the general answer, deferred until a column arrives with no lossless sentinel; the scoring floats the consumer keeps also need a float kind first.
* A query optimizer for aggregation (`GROUP BY`, `AVG`, multi-table joins) — DuckDB's core identity is vectorized execution over exactly this. A bounded `GROUP BY`/`COUNT`/`SUM`/`AVG`/`MIN`/`MAX` exists (`ADR-0035`) as a full scan then a bucket, with no optimizer of any kind — except, since `ADR-0081`, a `COUNT(*)` whose filter is only bounds on a range-indexed field, which is the sorted index's own count between the bounds with no record read (and, since `ADR-0082`, `SUM`/`AVG`/`MIN`/`MAX` of that field are reductions over the walked keys, no record read either; an aggregate over any other field still decodes; since `ADR-0084` a `GROUP BY` of that field over a pure range is one group per run of equal walked keys, and every other `GROUP BY` still decodes — since `ADR-0085` through a hashed bucket, linear in the rows).
* Client ecosystem — drivers for other languages, a CLI, general tooling. A byte-level wire specification and a stdlib-only Python client exist (`ADR-0043`); everything else on this list does not.
* Decades of hardening. SQLite's reliability record is the product of 20+ years and one of the largest test suites in software. This project's crash-safety work is real and genuinely tested, but young by comparison.

## Operational maturity — not named in this document before

Everything above is framed as "SQLite/DuckDB parity," which is really
about the query/transaction surface. A handful of things every
production DBMS needs were never named on this page at all — not
declined, just never asked. Recorded here for the same reason the rest
of this document exists: honest accounting before a future decision,
not a guess.

* **Backup/restore as a real, named operation.** *Partly built since
  this was written:* `Request::Backup`/`Response::BackedUp`
  (`ADR-0065`, protocol 24) is a real, in-process, lock-consistent
  snapshot copy under the table's existing write lock, path-confined
  to an opt-in `SERVER_BACKUP_ROOT`-configured server-local directory
  — every companion file copied by one shared prefix-glob
  (`copy_table_files`), write-to-a-temp-directory then one atomic
  rename so a hard kill never leaves a partial directory visible.
  Wired into `memory_server.rs` (the one binary with real
  `SERVER_DATA_DIR` durability); `DogConnectionStore::backup` is
  mechanically capable but no shipped binary calls it yet. Still
  absent: any wire `RESTORE` request or automated/live restore.
  **Now built**: a real offline CLI
  (`SERVER-RESTORE`, `ADR-0070`, `STORAGE-020` v0.1.0,
  `examples/restore_backup.rs`) — copies a `Request::Backup`-produced
  directory into a fresh target, crash-safely (staged, then each file
  renamed into place — per-file atomic, not one whole-group swap, a
  named limitation since the target may already hold sibling tables'
  own files), refusing an existing target outright, and verifies the
  result via a real reopen through the domain's own portable
  production constructor. Restore is architecturally an offline,
  directory-level, "copy then restart" operation for this crate's
  mmap-backed server model, not a live wire one — a `Request::Restore`
  would still need a subsequent process restart to take effect (see
  `ADR-0070`'s own Context), so this stays the whole story unless a
  future round finds a real need for wire-reachable restore.
* **Replication/high availability.** *Partly built since this was
  written:* `Request::FetchSnapshot`/`Response::Snapshot`
  (`ADR-0067`, protocol 25) streams a full, lock-consistent copy of a
  table's files back over the wire — `Backup`'s reading twin, reusing
  its `with_exclusive` lock and `copy_table_files` file-enumeration
  unchanged — gated behind a new, distinct `TokenClass::Replication`
  never satisfied by `ReadOnly`/`ReadWrite`; refuses `TooLarge` before
  any byte is read if the table exceeds `MAX_SNAPSHOT_BYTES` (8 MiB).
  Wired into `memory_server.rs` (`SERVER_AUTH_REPLICATION_TOKEN`);
  `DogConnectionStore::fetch_snapshot` is mechanically capable but no
  shipped binary calls it yet, the same standing gap `Backup` already
  has there. Still absent, explicitly: no continuous or incremental
  shipping — every fetch is a full snapshot, and the per-adapter
  crash-recovery journal (`ADR-0025`/`0026`/`0063`) stays unsuited to
  tailing, confirmed by reading it directly (aggressive
  checkpoint-then-truncate, no acknowledgment concept, entries only
  replay against an already-identical starting snapshot); no automatic
  failover or promotion — a refreshed replica is a manually-promoted
  cold standby only; no write forwarding — a replica never proxies
  writes to a primary; no cluster membership, gossip, or consensus; no
  replica-refresh daemon shipped by this crate — an operator's own
  script fetches, writes to local disk, and restarts a second
  `memory_server` pointed at it (`Backup`'s own "restore needs no new
  code" precedent).
* **Metrics/observability at the storage-engine layer.** *Partly built
  since this was written:* `Request::Metrics`/`Response::Metrics`
  (`ADR-0064`, protocol 23) renders a bounded, fixed set of
  process-wide atomic counters (`requests_total`, the ok/error split,
  connections accepted/active, uptime) as hand-formatted Prometheus
  text, gated as a read at the version gate. The audit log
  (`ADR-0029`) and access log (`ADR-0031`) still separately record
  admission/auth/request events to a file. Still absent: request
  latency histograms, queue depth, journal size, cache/index stats.
  **Now built**: an opt-in HTTP `/metrics` listener
  (`SERVER-METRICS-HTTP`, `ADR-0069`, `SERVER-001` v0.57.0/FR-069) —
  `SERVER_METRICS_HTTP_ADDR` binds a second, independent
  `TcpListener` answering a stock Prometheus scrape (`GET /metrics
  HTTP/1.1`) with the identical `ServerMetrics::render()` text,
  unchanged; absent by default, no endpoint auth in this round (a
  named, accepted tradeoff — see `ADR-0069`). `Metrics` over the
  existing binary wire protocol is untouched and still works exactly
  as before. (The Prometheus-text metrics the differential test suite
  exercises belong to the *consumer's* hub layer, `rusty_remind_me`,
  not this crate.) **Since `ADR-0086`**: one index stat —
  `dogserver_query_plans_total{plan="…"}`, the path each planned read
  took (`SERVER-001` v0.71.0/FR-083), classified before dispatch by a
  pure function of the request. **Since `ADR-0088`**: request latency —
  `dogserver_request_duration_seconds`, one histogram with sixteen
  fixed buckets (`SERVER-001` v0.73.0/FR-085), observed once per
  dispatched request; still absent: queue depth, journal size.
* **Schema migration tooling.** *Partly built since this was written:*
  a documented three-step pattern — a caller-defined old-layout struct
  implementing `SchemaTag` under the old tag; the existing
  `open_..._portable` read path, typed over it; the existing
  `create_..._production_stack` write path, unmodified, for a fresh
  new-tagged directory — proven end to end, as a real CLI
  (`examples/migrate_memory_v1_to_v2.rs`, `ADR-0066`,
  `STORAGE-019`), against `Memory`'s own `@1 → @2` bump (`ADR-0056`),
  the one bump this crate has ever shipped. Still absent: a reusable
  generic engine (deliberately declined — one real case, not enough to
  validate an abstraction against) and coverage for any *other* record
  type's tag bump, since none has ever happened — the next one still
  needs its own old-layout struct and conversion function written by
  hand, following this round's now-proven pattern rather than
  inventing it from scratch.

What's already solid and wouldn't need to be redone: the storage engine itself, real measured durability, real measured concurrency (single- and multi-process), and a working generic schema layer proven against more than one domain. SQL, transactions, and arbitrary joins would be built on top of that foundation, not require rebuilding it — but each is a serious, standalone effort, not a small extension of this project. The operational-maturity items above are a separate axis from SQL/transactions/joins entirely — a single-process, single-directory deployment can be genuinely production-hardened (backup, migration tooling, metrics) without ever growing a query planner or MVCC, and vice versa; neither axis is a prerequisite for the other.
