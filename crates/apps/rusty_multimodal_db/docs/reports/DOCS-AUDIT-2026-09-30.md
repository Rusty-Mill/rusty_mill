# Documentation audit — `rusty_multimodal_db`, 2026-09-30

Run with the `docs-loop` method: ground truth from the tree first, then the
docs, then a fix per row. Declared: the docs had already been read earlier in
the session that produced this audit (the growth-line work), so every claim
below was re-derived from the tree (`Cargo.toml`, `src/`, the engine crate,
the CI workflow, `check_references.py`), not from recall.

## Ground truth used

- `Cargo.toml`: features (`research`, `client`, `server`, `perf-events`),
  `publish = false`, `rust-version`, binaries and examples.
- The engine crate `crates/libs/storage/rusty_multimodal_db_engine` holds
  `generic/{traits,store,mmap_store,production,query,insert_log,record_blob,
  edge_blob,order_customer,mmap_scanned,slot_file,mmap_field}.rs` and
  `codec.rs`; `src/generic/mod.rs` re-exports it.
- `PROTOCOL_VERSION` 35 (`src/server/protocol.rs`, the Python client's
  `protocol.py`).
- Seven adapters in `src/server/` (`dog`, `order`, `employee`, `reminder`,
  `entity`, `memory`, `relation`); four server binaries.
- CI: the root workflow's `multimodal-db-feature-sets` job.
- `check_references.py`: 445 broken paths before, 274 after.

## Findings and what was done

| Class | Where | Finding | Action |
|---|---|---|---|
| stale | specs, `TRACEABILITY.md`, `SPEC-REGISTRY.md`, `README.md` | ~170 paths `src/generic/*.rs`, `src/codec.rs` name the pre-extraction location (`ADR-0124`) | rewritten to the engine crate; registry note added |
| stale | `README.md` | dependency snippet used the standalone repo's git URL and a sibling path; no mention of the engine crate | rewritten (path dependency in the workspace, engine crate) |
| stale | `README.md` | "wire protocol (currently version 30)" | 35 |
| stale | `README.md` | "Six domain adapters" (front-door three) | seven, front-door four (`Relation`) |
| missing | `README.md` | pages, `WriteBatch`, metrics, backup/replication, MVCC, planner, nullable, strict sessions, change log | one paragraph added, each with its ADR |
| missing | `README.md` | ADR-0053 to ADR-0133 not summarised | one bullet added; engine README linked |
| stale | `AGENTS.md` | "single crate at repo root, no workspace"; standalone-repo and `rustils` rules; `cargo ... --all-features` commands; "DogRecord's three fields are fixed, no generic schema" | rewritten to the monorepo layout and the commands CI runs; record-layout rule restated around schema tags |
| stale | `WORKFLOW.md` | "all four `STORAGE-*` units" (the registry has 21); no CI pointer | fixed; CI location added |
| stale | `clients/python/README.md` | "protocol version 18"; methods since 22 missing | 35; methods listed; sessions noted as not in this client |
| stale | `docs/architecture/SYSTEM-ARCHITECTURE.md` | pointer said six adapters, ADR-0010–0052; no layering | pointer updated; "Current layering" section added; `benches/workloads` path fixed |
| stale | `docs/PROJECT-STATUS.md` | header checkpoint was `af40ff3`, 2026-09-03 | new top checkpoint (`988369a`); old ones kept and labelled earlier |
| stale | `docs/FUTURE-GROWTH.md` | two test paths under `tests/` that live in the engine crate | fixed |
| stale | ADR-0130/0131/0133 titles | "(Proposal)" after acceptance | removed (status lines already accepted) |
| accurate | `STORAGE-011` `src/comparisons/` | a rejected non-goal, correctly names a path that does not exist | left |
| accurate | `README.md` binaries, features, security section, `ADR-0094` claim | checked against `Cargo.toml` and `src/bin` | left |
| historical | `ROADMAP.md`, dated `PROJECT-STATUS.md` entries, `RESULTS.md`, `docs/design/*`, older ADRs (~270 refs) | paths true when written | **left as written**, not drift |
| unverifiable | `WORKFLOW.md` | the `rust-repo-lifecycle` skill and its reference files | logged; nothing in the tree confirms them |
| unverifiable | `Cargo.toml` `repository` | still names the standalone repo | logged; a `Cargo.toml` change is not a docs edit |

## Verification

`check_references.py` after: 274 broken, all in the historical rows above
plus the one accurate non-goal. Commands the docs tell a reader to run and
that were executed: `python3 -m unittest discover -s clients/python/tests`
(10 pass), `cargo test -p rusty_multimodal_db --features client` (190 pass),
`RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps` (clean).
The `cargo bench` and `perf-events` commands were not run (they measure, and
the latter needs bare metal): unverified by design.
