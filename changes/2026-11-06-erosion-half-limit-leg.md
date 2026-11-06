---
category: Added
changelog: "`rusty_orch consolidate` also opens a D44 cleanup when erosion since the last cleanup reaches half of Nano's recorded limits."
---
## 2026-11-06 - Erosion gate: half-limit cleanups

- **Added:** D44's half-limit leg in `rusty_orch consolidate` (card #704 slice 4d, D114, D118, D120). It reads Nano's limits with `bbp erosion limits` and fires when either share has risen since the last cleanup's base by at least half its limit. With no record it is inactive and only the count leg runs. `--bbp-bin` now works in plan mode.
- **Docs:** orch ADR-0024; DECISIONS D120.
