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

One entry per merged PR against `main`, reverse chronological. No version tags
published yet (pre-1.0).

---

## Repo governance setup
**2026-08-25** · (not yet merged — link added once this lands)

- **Added:** applied the standard repo-config governance file set (README,
  ARCHITECTURE, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG,
  RELEASE_NOTES, ADR seed, issue/PR templates, `.gitattributes`,
  `ci-rust.yml`) — prerequisite for issue-loop to start working this repo's
  open issue backlog.
- Filled README/ARCHITECTURE with real content (boundary table, data flow)
  rather than leaving the scaffold placeholders.
- **Fixed:** `cargo fmt`/`cargo clippy -D warnings` baseline failures (unformatted
  code, `Framebuffer::present`'s `window` param unused off-Windows) — pre-existing,
  surfaced by the new `ci-rust.yml` gate; fixed so the "on green CI, merge" rule
  has a working baseline to gate on.
