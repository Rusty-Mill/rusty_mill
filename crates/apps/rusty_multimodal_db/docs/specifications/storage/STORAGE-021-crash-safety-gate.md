# STORAGE-021 — Crash-safety gate (the harness's trials as CI tests)

- Version: 0.1.0
- Status: Accepted, Implemented, Verified
- Owners: baileyrd
- Depends on: `STORAGE-012` (`GenericMmapStore::open`'s reconciliation),
  `STORAGE-017` (the trailing `COMMITTED` marker), the crash-safety
  diagnosis round (`src/bin/crash_safety_harness.rs`,
  `src/bin/crash_writer.rs`), `ADR-0095`.
- Supersedes: none. Adds no on-disk format, changes no code under
  `src/`.

## Purpose and scope

The four crash-safety trials the diagnosis harness runs by hand,
asserted as one integration test target that `cargo test
--all-features` runs on every push. Full design:
`docs/design/STORAGE-CRASH-SAFETY-GATE-DESIGN.md`.

## Requirements

- `STORAGE-021-FR-001` (`CSC-FR-001`): every update `Flush` returned
  for survives a `SIGKILL` and a cold reopen.
- `STORAGE-021-FR-002` (`CSC-FR-002`): a slot torn after its id or
  after its value is excluded on reopen and reseeded from the caller's
  record; the uninterrupted control keeps the attempted value.
- `STORAGE-021-FR-003` (`CSC-FR-003`): a torn in-place update reads as
  exactly one of the two written patterns.
- `STORAGE-021-FR-004` (`CSC-FR-004`): an unflushed kill leaves no
  record with a value it was never given; the survivor count is
  reported, not asserted.
- `STORAGE-021-FR-005` (`CSC-FR-005`): registered with
  `required-features = ["research"]` so CI's `--all-features` test job
  runs it; Unix only.
- `STORAGE-021-FR-006` (`CSC-FR-006`): nothing under `src/` changes.

## Acceptance criteria

`tests/crash_safety.rs`: four tests, green, about two seconds.

## Verification plan

As `docs/design/STORAGE-CRASH-SAFETY-GATE-DESIGN.md`.

## Traceability

`ADR-0095`; roadmap `STORAGE-CRASH-SAFETY-GATE`;
`docs/traceability/TRACEABILITY.md`.

## Change history

- 0.1.0 (2026-09-21, ADR-0095): initial, alongside `tests/crash_safety.rs`.
