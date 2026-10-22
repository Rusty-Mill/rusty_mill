---
category: Added
changelog: BBP moderators admit harnesses and runners through shared, crash-safe slots, and Orch passes its configured slots to every moderator it starts.
---
## 2026-10-21 - Slot admission for hosted work (Stage 3a)

- **Added:** `bbp mod --slots DIR --agent-slots N --validator-slots M`. A harness starts only with a free agent slot and a runner only with a free validator slot, across every moderator naming `DIR`; a turn or run without one waits. A slot is a kernel lock handed to the child as stdin (harness) or stdout (runner), so it is held until the child's whole process tree has exited, even when the moderator is killed first. The first moderator records the capacity; a differently configured one is refused.
- **Added:** optional `slots: {dir, agents, validators}` in Orch's BBP configuration, passed to each moderator the supervisor starts.
- **Tests:** four racing `bbp mod` processes never exceed two agents and one validator; a killed moderator's orphaned harness keeps its slot until it exits, and the restarted moderator then reclaims it. Acceptance mode `slot-admission` runs two Orch goals through one agent slot.
