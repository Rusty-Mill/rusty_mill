# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

No version tags yet (pre-1.0, nothing published). Tracked by unit of change,
reverse chronological, each linking to its PR once one exists.

---

## ADR-0002: Phase 1 foundational decisions
**2026-08-01**

- **Added:** `docs/adr/0002-phase1-foundational-decisions.md` — resolves all four
  `docs/phase1-scope.md` §6 open questions with cited research: skip Kafka
  wire-protocol compatibility (build on `rusty_wire`); defer the VSR-vs-Raft
  choice to Phase 2 but lean VSR and require consensus-ready storage primitives
  now (durable/committed-offset split, truncatable log tail, epoch/fencing-token
  field); compio as a provisional runtime choice pending a validation spike;
  coexist with NATS JetStream behind an explicit, criteria-based re-evaluation
  gate rather than replacing it outright.
- **Added:** a concrete DST testing strategy (injectable `Storage`/`Clock`
  traits from the first storage-engine commit, three minimal fault-injection
  tests) and a set of "consumer gates" the storage engine must clear before
  Phase 1 is considered done.
- **Known limitation:** the runtime choice (D3) is explicitly provisional —
  the ADR names a validation spike that hasn't run yet. No implementation
  lands in this change; this is research/ADR only, per the scope doc's gate.

## Repo setup — minimal CI workflow + main branch
**2026-08-01**

- **Added:** `main` branch on `origin`, created from the governance-scaffolding
  commit and now the repo's default branch.
- **Added:** `.github/workflows/ci.yml` — a minimal `check` job (name matches the
  required-status-check convention in the repo-config reference) that no-ops green
  until a `Cargo.toml` exists, then automatically runs `cargo fmt`/`clippy`/`test`.
  Exists now so branch protection has a real check to gate on rather than one
  that's never reported.
- **Known limitation:** branch protection on `main` is still unset — no tool in
  this environment reaches GitHub's branch-protection or repo-settings API, so
  that (require PR, require the `check` status, require up-to-date branches, and
  disabling squash/rebase merge in repo settings) remains a manual step.

## Repo setup — Phase 1 scope doc + governance scaffolding
**2026-08-01**

- **Added:** trimmed copy of the Phase 1 pre-RFC research brief
  (`docs/phase1-scope.md`) — dropped the "governance-native data contracts"
  differentiator per direction to leave that out for now.
- **Added:** standard governance file set via `repo-config` — README, ARCHITECTURE,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, PR/issue templates, ADR seed.
- **Known limitation:** no `Cargo.toml` yet, so no CI workflow was added — nothing
  to run. `ARCHITECTURE.md`'s boundary table is left as scaffold since no code has
  landed. This repo has no `main` branch yet (root commit lives on
  `claude/review-attached-document-3at8q9`), so the PR-per-change workflow doesn't
  apply until one exists.
