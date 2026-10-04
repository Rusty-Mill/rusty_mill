# AGENTS.md

## Scope

Applies to the `rusty_multimodal_db` crate (`crates/apps/rusty_multimodal_db`)
inside the `rusty_mill` monorepo. The monorepo's own conventions (root
`AGENTS.md`/`CLAUDE.md`, `.github/`) apply on top.

## Project shape

- Purpose: began as an empirical benchmark of three storage-layout backends
  (AoS, SoA, UUID-canonical-store-with-views) behind one `DogStore` trait,
  to decide whether a UUID-canonical store can serve row/column/graph access
  as views rather than physical copies (`docs/charter/CHARTER.md`,
  `RESULTS.md`). The winner became `ProductionStore` /
  `GenericProductionStore`, and around it grew a network server/query layer,
  the domain adapters a consumer (`rusty_remind_me`) uses, and clients.
- Rust structure: this crate is one member of the monorepo workspace
  (layer `apps`). Its generic storage core lives in the libs crate
  `crates/libs/storage/rusty_multimodal_db_engine` (`ADR-0124`) and is
  re-exported here as `rusty_multimodal_db::generic::*`, so a change to
  `generic/{traits,store,mmap_store,production,query,...}.rs`, `codec.rs`,
  the journal, the full-text index or the directory lock is made in the
  engine crate, and its tests run there. Here: `src/` (`record`,
  `generator`, `store` + the research backends, `durability`, `concurrency`,
  `production`, `generic/{entity,memory,relation,reminder}.rs` (and `generic::fair_play`, a re-export of the libs crate `rusty_fair_play_domain`), `server/`,
  `bin/`), `benches/`, `tests/`, `examples/`, `clients/python/`.
- Features: default (front-door only: `ProductionStore`,
  `GenericProductionStore`, the domain stacks), `research` (the benchmarked
  alternatives and spikes), `client` (the wire client and protocol),
  `server` (implies `client`; the listener and adapters), `perf-events`.
- Architectural boundaries: see
  `docs/architecture/SYSTEM-ARCHITECTURE.md` — one-way dependency
  direction (backends depend on `store`/`record`, never the reverse), and
  the "views, not copies" boundary for `CanonicalStore` (ADR-0001).

## Coordination

Follow `WORKFLOW.md` for handoffs and review — it governs process, not
project architecture.

## Canonical commands

Scope every command to the crate (`-p rusty_multimodal_db`); CI runs the
feature sets separately (`ADR-0098`), so check the ones you touched:

- Format: `cargo fmt -p rusty_multimodal_db -- --check`
- Lint: `cargo clippy -p rusty_multimodal_db --all-targets -- -D warnings`,
  repeated with `--features client`, `--features server`,
  `--features research` and `--features server,research`. A client-only
  build must stay warning-free: gate server-only items behind
  `#[cfg(feature = "server")]`.
- Test: `cargo test -p rusty_multimodal_db --features server,research`
  (plus `--features client` for the client half alone); the Python client:
  `python3 -m unittest discover -s clients/python/tests`.
- Docs: `RUSTDOCFLAGS="-D warnings" cargo doc -p rusty_multimodal_db
  --all-features --no-deps` (`ADR-0101`).
- Benchmarks (wall-clock, cross-platform): `cargo bench -p
  rusty_multimodal_db --features research`
- Benchmarks (cache-miss, Linux bare-metal only): `cargo bench --features
  perf-events --bench cache_events` — see ADR-0002 before relying on this
  anywhere else.
- After any dependency or feature change: regenerate the workspace map the
  way CI checks it — `cargo metadata --format-version=1 --all-features
  --locked > /tmp/metadata.json`, then `python3
  .github/scripts/generate_workspace_map.py /tmp/metadata.json >
  docs/WORKSPACE-MAP.md` (repo root).

## Change rules

- `Result` + `?` over panics; `unwrap()`/`expect()` only inside `#[cfg(test)]`.
- Tests for all non-trivial logic: happy path plus at least one
  boundary/failure case.
- Docstrings (`///`) on public functions/structs/modules — this repo is
  past the initial spike as of the `GENERATOR` unit onward.
- No speculative generality: `DogRecord`'s three fields are fixed. The
  generic schema library exists (`ADR-0009`, accepted) but a new value kind,
  column type or wire variant needs its own ADR. A domain record's layout is
  versioned by its schema tag (`ADR-0056`); changing one is hard to reverse
  and needs a migration path (`STORAGE-019`, `ADR-0066`), so ask first, as
  for any wire change (a protocol bump is append-only, `SERVER-002` §8).
- New dependencies must be justified in one line in the commit message or
  an ADR. Prefer the standard library; check `docs/decisions/` before
  assuming a crate is needed if something similar was already evaluated.
- The monorepo's dependency policy applies (`dependency policy (workspace
  sovereignty)` in CI: `.github/scripts/check_workspace_deps.py`, the layer
  check and the workspace-map check); a new dependency needs to pass it.
- Flat control flow, guard clauses over deep nesting.
- Update `docs/PROJECT-STATUS.md`, the roadmap, and
  `docs/traceability/TRACEABILITY.md` when a roadmap unit's state changes.

## Definition of done

- Tests pass, lints clean (`-D warnings`) and formatting clean for every
  feature set you touched, as listed under Canonical commands.
- Public items documented.
- Roadmap/spec-registry/traceability/status updated in the same PR (or an
  immediate follow-up) when the unit changes state.
- An ADR exists for any new consequential design choice per
  `docs/decisions/` cadence guidance (this project is in active
  bootstrap/major-development, so default to writing one — ADRs are
  append-only: supersede rather than edit; see also `ADR-0062`, the
  definition of done).
