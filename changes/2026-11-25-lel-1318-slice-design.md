---
category: Added
changelog: Let Em Loose: #1318 builder-launch slice design (six stacked slices, then the D167 toy run) and D187.
---
## 2026-11-25 - Let Em Loose: #1318 builder-launch slice design

- **Added:** `docs/projects/let-em-loose/1318-builder-launch-slices.md`, the draft slice plan for orch launching #600 builders as mux-hosted jailed agents: the fd hand-off and spawn helpers, the holder taking its slot, an ACP client with the `acp.log` renderer, the `rusty-mux-ctl own` adapter, the launch sequence with `rusty_orch builder run`, and the §11 tests in CI. D187 records the expert's implementation defaults.
- **Known limitations:** a draft for Nano's approval; one open point (how a candidate reaches BBP) blocks the launch-sequence slice.
