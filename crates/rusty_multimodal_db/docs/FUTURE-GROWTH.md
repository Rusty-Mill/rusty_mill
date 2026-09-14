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

1. SQL. A parser, a query planner, a cost-based optimizer, and an execution engine. Every query today is a hand-written Rust method call against a specific schema's traits — there's no declarative language layer at all. *Partly built since this was written:* a client-side `SELECT`/`GROUP BY`/`JOIN`/`ORDER BY` subset (`ADR-0034`/`0035`/`0044`/`0061`) compiled to fixed request shapes, and one ordered keyset page on the wire (`ADR-0055`) — `ORDER BY` in the SQL text (`ADR-0061`) compiles to it directly, for a plain query only (not combined with `WHERE`/`JOIN`/`GROUP BY` — filtering-then-ordering in one request is still absent). Still absent: any query planner or optimizer, and `ORDER BY` combined with a filter.
2. Transactions. *Partly built since this was written:* a batch of field updates applies all-or-nothing (`ADR-0013`), a per-connection session stages writes and commits them as one batch with read-your-writes and snapshot isolation (`ADR-0024`/`0027`/`0033`), an opt-in redo journal makes a `Request::Transaction` batch crash-atomic (`ADR-0025`), and — since `ADR-0063` — the same is now true of `Request::WriteBatch`'s atomic mode on `Memory`/`Entity`/`Relation` (journal format version 2, when the adapter is journaled). Still absent: a general transaction manager — real MVCC (versioned records, a snapshot pointer, garbage collection — `ADR-0033`'s own explicitly-declined option), multi-table atomicity, and staged (not single-shot) transactions over inserts, links, replacements, and deletes.
3. Arbitrary joins. *Partly built since this was written:* a `JOIN … ON <relation>` over a declared relation, within a table or across two tables on one connection (`ADR-0044`/`ADR-0050`), evaluated as an index nested loop. Still absent: a join on an arbitrary predicate at query time — every relation is still declared ahead of time, and relation labels created at runtime (`ADR-0047`) are labels on a declared relation kind, not new predicates.

Smaller, but still real:

* Dynamic/runtime schema (`ALTER TABLE`-style changes). Schema here is a compile-time Rust concept, and a record layout is unversioned: adding a field is a schema-tag bump that refuses old directories distinctly (`ADR-0056`), with an in-place upgrade named as the round for the first directory that cannot be re-pushed.
* Null. The wire has no null; two domains carry documented per-field sentinels instead (`ADR-0056`: `0` for a timestamp, `""` for a node id — lossless for those columns). A nullable `ScanValue` at a protocol bump is the general answer, deferred until a column arrives with no lossless sentinel; the scoring floats the consumer keeps also need a float kind first.
* A query optimizer for aggregation (`GROUP BY`, `AVG`, multi-table joins) — DuckDB's core identity is vectorized execution over exactly this. A bounded `GROUP BY`/`COUNT`/`SUM`/`AVG`/`MIN`/`MAX` exists (`ADR-0035`) as a full scan then a bucket, with no optimizer of any kind.
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
  absent: any `RESTORE` request or documented restore procedure — a
  backup is a portable, copy-safe directory (`STORAGE-014`–`016`), so
  "restore" today means manually pointing a server's data directory at
  the backup and restarting it, not a request or tool that does that
  for you.
* **Replication/high availability.** Zero — no primary/replica concept,
  no write-ahead shipping beyond the per-adapter crash-recovery journal
  (`ADR-0025`/`0026`/`0063`, which never leaves the one process), no
  failover. A single process owning a single directory is the only
  deployment shape this crate has ever had.
* **Metrics/observability at the storage-engine layer.** *Partly built
  since this was written:* `Request::Metrics`/`Response::Metrics`
  (`ADR-0064`, protocol 23) renders a bounded, fixed set of
  process-wide atomic counters (`requests_total`, the ok/error split,
  connections accepted/active, uptime) as hand-formatted Prometheus
  text, gated as a read at the version gate. The audit log
  (`ADR-0029`) and access log (`ADR-0031`) still separately record
  admission/auth/request events to a file. Still absent: request
  latency histograms, queue depth, journal size, cache/index stats, or
  any HTTP `/metrics` endpoint — `Metrics` is answered over the
  existing binary wire protocol, not scraped by Prometheus directly.
  (The Prometheus-text metrics the differential test suite exercises
  belong to the *consumer's* hub layer, `rusty_remind_me`, not this
  crate.)
* **Schema migration tooling.** `ADR-0056`'s schema-tag versioning
  refuses a directory built under an older layout by name, distinctly —
  a real safety property — but there is no migration *runner*: no
  command that reads an old-tag directory and rewrites it under the new
  one. Today a layout change is a manual, per-deployment operation.

What's already solid and wouldn't need to be redone: the storage engine itself, real measured durability, real measured concurrency (single- and multi-process), and a working generic schema layer proven against more than one domain. SQL, transactions, and arbitrary joins would be built on top of that foundation, not require rebuilding it — but each is a serious, standalone effort, not a small extension of this project. The operational-maturity items above are a separate axis from SQL/transactions/joins entirely — a single-process, single-directory deployment can be genuinely production-hardened (backup, migration tooling, metrics) without ever growing a query planner or MVCC, and vice versa; neither axis is a prerequisite for the other.
