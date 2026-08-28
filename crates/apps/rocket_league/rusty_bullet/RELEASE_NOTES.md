# Release Notes

Tracks notable changes to this repo, one entry per merged change against
`main`, reverse chronological. Pre-1.0, no version tags yet — entries are
keyed by the commit/PR that shipped them.

---

## Repo bootstrap — full lifecycle baseline
**2026-08-28** · commit on `claude/rocket-league-server-clone-u74q45` (not yet merged; link added once merged)

- **Added:** Full `rust-repo-lifecycle` + `repo-config` bootstrap: charter,
  system architecture, a 6-spec tree (`RB-VERIFY-001/002/003` fully
  specified for Phase 0; `RB-PHYSICS-001`/`RB-SIM-001`/`RB-NET-001` as
  forward-looking placeholders), 3 ADRs (server-authoritative netcode,
  verification-first ordering, Bullet-fidelity target), a research backlog
  (6 settled findings + 3 tracked open questions), a Phase 0-4 roadmap with
  exit criteria tied to the divergence metric, requirement-level
  traceability, AGENTS.md/WORKFLOW.md, and the standard governance file set
  (README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, PR/issue
  templates).
- **Added:** Minimal buildable Cargo workspace — `rb_domain` (physics frame
  types, `PhysicsStateSource` port, divergence-scoring algorithm),
  `rb_replay_ingest`/`rb_capture_ingest` (adapter stubs implementing the
  port), `rb_verify_cli` (composition-root binary). Divergence scoring
  (`RB-VERIFY-003-FR-001`) is real and unit-tested; both ingestion adapters
  are intentionally stubbed (`IngestError::NotImplemented`) — `boxcars`
  parsing and the BakkesMod capture format are Phase 0 delivery work, not
  bootstrap scaffolding.
- Known scope cut, stated plainly: no physics/simulation/netcode code
  exists yet — this PR is the governed baseline the rest of the project
  builds against, per ADR-0002's verification-first ordering.
- 11 unit tests added (6 in `rb_domain`, 1 each in the two adapter stubs,
  plus workspace doc-tests); `cargo fmt --check`, `clippy -D warnings`, and
  `cargo test --workspace` all pass.
