---
category: Added
changelog: Findings from the unimported rusty_bullet server-clone branch mapped to main's RB-PHYSICS-001 requirements, plus the capture defects it found.
---
## 2026-10-24 - Rocket League: server-clone branch findings mapped to main

- **Added:** `rusty_bullet/docs/research/SERVER-CLONE-BRANCH-FINDINGS.md`. The branch reused `RB-PHYSICS-001-FR-079` to `FR-094` for different findings; each is now mapped by topic to the main requirements that cover the same mechanism, with main's error on the branch's own capture as evidence. None needs new code except the branch's open finding J, which is only partly investigated: reseeded at 4.0 s, before its bounce, main ends upright like the recording instead of inverted, but its trajectory still diverges (231 uu max), so J stays open.
- **Added:** the capture defects the branch found (unlimited boost in the September freeplay sessions, single-tick spin spikes and input drops, steer, pitch, yaw and roll recorded as zero in two clips, a missing `jump` press, seeding inside a held jump), linked from `RB-VERIFY-002` (0.4.1).
- **Known limitations:** the mapping is by mechanism, not line by line against the branch's code; the evidence column is what settles each row.
