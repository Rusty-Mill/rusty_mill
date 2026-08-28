# AGENTS.md

## Scope

Applies to the whole repository. There is no nearer `AGENTS.md` yet; add one
under a crate only if that crate develops rules genuinely different from
these.

## Project shape

- Purpose: reimplement Rocket League's client/server physics and netcode
  architecture in Rust, staged behind a verification pipeline that scores
  every later phase against recorded ground truth. See
  [README.md](./README.md) and
  [docs/architecture/SYSTEM-ARCHITECTURE.md](./docs/architecture/SYSTEM-ARCHITECTURE.md).
- Rust structure: single Cargo workspace, modular monolith. Crates split at
  real responsibility boundaries only:
  - `crates/rb_domain` — pure domain logic (physics frame types, divergence
    scoring, ports). No I/O, no third-party dependencies unless a second
    real call site justifies one.
  - `crates/rb_replay_ingest` — adapter implementing `PhysicsStateSource`
    over replay files (`boxcars`, once wired up).
  - `crates/rb_capture_ingest` — adapter implementing `PhysicsStateSource`
    over BakkesMod offline captures.
  - `crates/rb_verify_cli` — composition root binary (`rb-verify`); wires
    adapters to domain logic, no domain logic of its own.
- Architectural boundaries: domain crates never depend on adapter crates;
  adapters depend on `rb_domain`, never on each other. New I/O (a new file
  format, a new capture source, a physics engine binding) gets its own
  adapter crate implementing an existing or new domain port — it does not
  get bolted onto `rb_domain`.

## Coordination

Follow [WORKFLOW.md](./WORKFLOW.md) for handoffs and review — it governs
process, not project architecture.

## Canonical commands

- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Test: `cargo test --workspace`
- Docs/build: `cargo doc --workspace --no-deps` / `cargo build --workspace`

## Change rules

- Prefer `Result` + `?` over `panic!`/`unwrap`/`expect` outside tests
  (`clippy::unwrap_used`/`expect_used`/`panic` are `warn`-level workspace
  lints — treat a new warning as a defect, not noise to suppress).
- Composition over inheritance; no trait-object hierarchies standing in for
  a single concrete implementation.
- No speculative abstraction before two real call sites exist. The
  `PhysicsStateSource` port exists because two adapters need it today — the
  same bar applies to any new trait.
- Every new third-party dependency needs a one-line justification in the
  PR description (why it's needed, why hand-rolling isn't better here).
  `rb_domain` in particular should stay dependency-free as long as possible.
- Tests required for all non-trivial logic: happy path plus at least one
  boundary/failure case. A stub adapter's "not implemented yet" behavior
  still gets a test (see `rb_replay_ingest`/`rb_capture_ingest`) so it fails
  loudly instead of silently succeeding once real logic replaces the stub.
- Update `docs/roadmap/ROADMAP.md`, `docs/specifications/SPEC-REGISTRY.md`,
  and `docs/traceability/TRACEABILITY.md` when a roadmap unit's status
  changes or a spec's implementation/verification state moves.
- Write an ADR (`docs/adr/`, template in `docs/adr/TEMPLATE.md`) for any
  cycle that chooses between real alternatives, changes a public
  interface/data format, or would be expensive to reverse later — see
  `docs/adr/` cadence note in [WORKFLOW.md](./WORKFLOW.md). This project is
  in active major development, so that cadence is roughly one ADR per
  delivery cycle, not just for rare forks.

## Definition of done

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` all pass.
- New/changed public behavior has tests and doc comments.
- Roadmap/registry/traceability updated if the unit's status changed.
- PR uses the matching template under `.github/PULL_REQUEST_TEMPLATE/` and
  fills in the acceptance-criteria mapping.
