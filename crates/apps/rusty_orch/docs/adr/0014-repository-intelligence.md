# ADR-0014: Read-only repository intelligence reuses CI policy

- **Status:** Accepted for Milestone 1
- **Date:** 2026-10-18

## Context

Repository discovery, dependency-layer checks and reverse impact analysis already
exist in the monorepo's CI scripts. Reimplementing their policies in the Orch
domain would create competing definitions and violate its pure-core boundary.
The root architecture overview still calls ADR-0003 proposed; the implemented
ADR and existing validator define the current five-layer policy.

## Decision

Add `rusty_orch inspect [--repo <dir>]` at the command composition root. It emits
versioned JSON and never enters the dispatcher, opens a state store, selects a
provider or creates a BBP task. Goal execution and the BBP protocol are unchanged.

Bundle the existing `affected_crates.py`, `check_workspace_layers.py` and
`check_workspace_deps.py` sources into the binary and load them into isolated
Python modules. This reuses the exact first-party functions without executing
scripts from the target checkout or introducing a third-party Rust dependency.
Python 3.11+ is an explicit optional runtime prerequisite for inspection; its
standard library supplies JSON, hashing and TOML. Existing commands do not
require Python. This monorepo builds the binary; packaging it as a standalone
Cargo source package is outside this increment because the bundled CI files
live outside the crate directory.

The existing `orch-cli::CommandRunner` bounds execution to 120 seconds, kills the
process group on failure/timeout, and limits report stdout to 1 MiB. Python runs
Cargo metadata with a 90-second inner deadline, `--all-features --frozen`, no
shell, builds or tests. A current lockfile and cached dependencies are required.
Missing prerequisites and incomplete metadata fail inspection, never return a
clean report. Repository Cargo configuration and executables on PATH remain
trusted local tooling, as for normal Cargo use.

Resolved edges include dev, build, optional and target-specific relationships;
impact is a conservative transitive reverse closure, not a prediction of edit
size or runtime risk. Deterministic findings cover invalid layer classification,
forbidden layer/family edges, first-party git shadows, pinned external dependency
policy failures and identical explicit external dependency declarations.
Duplicate declarations are advisory: inherited/path dependencies are excluded,
and version, features, source, dependency kind and target must match exactly.

## Consequences

The JSON is a future task-intake artifact, not a new protocol or task lifecycle.
No automatic task assignment or modification is implemented. Static checks do
not claim that builds, tests or live CI passed. The schema and prerequisites are
documented in the command README; unit tests exercise policy reuse and Rust
integration tests exercise real Cargo discovery and read-only failure behavior.
