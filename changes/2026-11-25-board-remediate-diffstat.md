---
category: Added
changelog: Mill Board Remediate cards now show the dry-run diffstat of the acted-on work they undo (files, +/- lines, largest files); card #1400.
---
## 2026-11-25 - Mill Board: Remediate cards carry the diffstat

- **Added:** each dropped acted-on claim on a Remediate card shows its candidate's diffstat from the host's `acted[].diffstat` (#1869): `3 files, +41 -7: `src/a.rs` +30 -2, ... and 1 more`. Paths are agent-written, so control characters and backticks are replaced and the line cannot break the card. A claim with no readable candidate, or a host that predates the field, shows a "No diffstat" line instead of failing.
- **Known limitations:** the real-bbp CI lane has no acted-on claim scenario (the CLI cannot mark a claim acted or store a candidate; it needs the MCP driver), so the lane does not assert the diffstat yet. The fake-host card test and a wire-shape test cover it.
