---
category: Added
changelog: Findings from the unimported rusty_bullet server-clone branch mapped to main's RB-PHYSICS-001 requirements, plus the capture defects it found.
---
## 2026-10-24 - Rocket League: server-clone branch findings mapped to main

- **Added:** `rusty_bullet/docs/research/SERVER-CLONE-BRANCH-FINDINGS.md`. The branch reused `RB-PHYSICS-001-FR-079` to `FR-094` for different findings; each is now mapped by topic to the main requirements that cover the same mechanism, with main's error on the branch's own capture as evidence. Finding J stays partly investigated/open: the 4.0 s reseed precedes its 4.575 s bounce/dodge and ends upright, but its trajectory still diverges (231 uu max) and its seed alignment is unverified. The 6.2 s reseed only demonstrates later-state recovery. Closure requires the verified original pre-event reproduction and outcome through 6.4–6.7084 s.
- **Added:** the capture defects the branch found (unlimited boost in the September freeplay sessions, single-tick spin spikes and input drops, steer, pitch, yaw and roll recorded as zero in `hittickjump01`/`01b` (throttle is nonzero in 442/445 frames, respectively), a missing `jump` press, seeding inside a held jump), linked from `RB-VERIFY-002` (0.4.1).
- **Known limitations:** the mapping is by mechanism, not line by line against the branch's code, and bounded captures do not close every finding or prove that no further code is needed. The `hit-tick-jump` ball error remains unresolved (188.57 uu mean / 1023.82 max). This documentation depends on corrected #986 merging first; no new physics code is included.
