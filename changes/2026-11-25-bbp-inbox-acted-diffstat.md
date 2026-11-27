---
category: Added
changelog: `bbp inbox --json` now gives each acted-on claim an optional `diffstat` of its current candidate, for Mill Board Remediate cards; card #1400.
---
## 2026-11-25 - BBP inbox: acted-on claims carry a diffstat

- **Added:** each `acted` row of `bbp inbox --json` has `diffstat`: files, added and removed lines, and the 10 largest files (paths cut at 200 characters, the rest counted in `more`), counted from the claim's current candidate's stored diffs in apply order. It is `null` when the claim has no candidate or a diff is unreadable. Parsed from the stored diff text: no git process, no checkout. Additive output field; no log or schema change, and readers that ignore unknown fields are unaffected.
- **Known limitations:** Mill Board does not render it yet (next slice of card #1400).
