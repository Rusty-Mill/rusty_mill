---
category: Changed
changelog: Rocket League netcode docs mark two GDC-talk claims that are not on the slides as unverified hypotheses.
---
## 2026-10-24 - GDC netcode claims marked as hypotheses

- **Changed:** ADR-0001, RB-NET-001 and the research backlog no longer state as fact that Rocket League sends periodic snapshots rather than full state, or that rubber-banding is reconciliation smoothing rather than latency. Neither is on the GDC 2018 slides; both come from a third-party summary of the talk video. They are now ADR-0001 hypotheses H1 and H2. The decision itself rests on what the slides do say and is unchanged.
- **Known limitations:** nobody has checked the spoken talk. The caption-only physics numbers in rl-physics-sources.md were already flagged and are not changed here.
