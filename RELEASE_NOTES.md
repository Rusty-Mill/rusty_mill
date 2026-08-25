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

One entry per merged PR against `main`, most recent first, each linking to its PR.

---

## PR #5 — Add repo-config governance files; fix pre-existing clippy warnings

**2026-08-25** · [#5](https://github.com/baileyrd/rusty_gui/pull/5)

- **Added:** standard governance file set (README, CONTRIBUTING, CODE_OF_CONDUCT,
  SECURITY, CHANGELOG, RELEASE_NOTES, ARCHITECTURE, an ADR seed, issue/PR
  templates, `.gitattributes`, and a `ci-rust.yml` GitHub Actions workflow).
- **Fixed:** two pre-existing `cargo clippy -D warnings` failures
  (`unused_mut` in `Window::poll_events`, `dead_code` on the placeholder
  `x11_window` field) that would otherwise have kept the new CI workflow red
  on every PR, including this one.
- No behavior change to the crate's public API.
