---
name: Parity gap
about: A capability present in rustils (or another reference) but missing/incomplete here
labels: parity-gap
---

## Gap

`{{SYMBOL}}` ({{CATEGORY}}) — present in {{REFERENCE}}, missing (or incomplete)
in this crate.

**Source:** {{SOURCE}} <!-- roadmap / diff / spec, from gap-analysis.md -->
**Platforms:** {{PLATFORMS}}
**Existing RustyMill impl:** {{RUSTYMILL_IMPL}} <!-- sibling repo, or "none found" -->
**Breaking change required:** {{BREAKING}}

## Why

{{NOTES}}

## Reference

{{REFERENCE_LINK}}

## Acceptance

- [ ] Implemented — ported and adapted from the RustyMill sibling above if
      one was found, otherwise written fresh — matching the reference's
      documented behavior for the platform(s) above
- [ ] Tests: happy path + boundary/failure cases
- [ ] Doc-comment on the new public surface
- [ ] `cargo clippy -- -D warnings` and `cargo fmt --check` clean
- [ ] `RELEASE_NOTES.md` entry added

<!--
Filed by the parity-loop skill from gap-analysis.md. If BREAKING is "yes",
this issue is a stop-and-ask, not an auto-implement. If SOURCE is "roadmap",
this issue traces back to an existing hand-curated scope doc (README's
"Reserved, not built" table / docs/adr/0001), not an independently-invented
one. If RUSTYMILL_IMPL names a repo, copying the code in is fine unattended;
depending on that repo as a crate is its own stop-and-ask.
-->
