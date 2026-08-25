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

Tracks notable changes to `rusty_font`, one entry per merged PR against
`main`, reverse chronological (no version tags yet, so PRs are the unit of
change).

---

## PR — Add standard governance files
**2026-08-25**

- **Added:** repo-config's standard governance set — PR/issue templates,
  CONTRIBUTING/CODE_OF_CONDUCT/SECURITY/CHANGELOG/RELEASE_NOTES/ARCHITECTURE,
  an ADR seed, `.gitattributes` (forces LF), and a Rust CI workflow
  (`fmt` + `clippy -D warnings` + `test`). README was left as-is (already
  present). ARCHITECTURE's boundary table and data-flow were hand-written
  for this crate's actual parse → outline → rasterize pipeline rather than
  left as scaffold.
- **Changed:** ran `cargo fmt --all` across the existing source
  (`src/ttf.rs`, `src/rasterizer.rs`, `examples/ascii_render.rs`) —
  formatting only, no behavior change — so the new CI's `fmt --check` gate
  doesn't start out red against unformatted pre-existing code.
- 15 unit tests unaffected: 15 passed, 0 failed.
- Link will be added once the PR is open.
