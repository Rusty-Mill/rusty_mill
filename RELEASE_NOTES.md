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
