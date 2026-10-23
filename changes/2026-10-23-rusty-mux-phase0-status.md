---
category: Changed
changelog: rusty_mux project docs record Phase 0 engine status (#872, #894, #930, #936, #939) and the owner's acceptance of #930.
---
## 2026-10-23 - rusty_mux Phase 0 status

- **Changed:** `docs/projects/mux/` capabilities, roadmap, risks and decisions now reflect the merged engine work: benchmark gate, wcwidth widths, DECCOLM, ring-buffer scroll, ASCII fast path and the snapshot serializer. Fidelity is 15/18 against the oracle, with the other three deliberate.
- **Known limitations:** the MB/s floors (#889) and a reliable ≤ 11.2 ms snapshot encode are not met.
