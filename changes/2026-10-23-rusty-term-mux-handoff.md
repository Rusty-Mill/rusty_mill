---
category: Added
changelog: Handoff notes for rusty_term's performance work (decisions, design context, remaining work and roadmap) under docs/projects/mux.
---
## 2026-10-23 - rusty_term handoff notes for the mux project

- **Added:** `docs/projects/mux/RUSTY-TERM-NOTES.md`: the decisions from the rusty_term performance work with their reasons and owner confirmation, the design context a multiplexer depends on (core crate boundary, snapshot format, DECCOLM, widths, BCE, lock and frame costs, PTY), open questions and references. It flags where rusty_term touches the mux constraints: the front-end crate's tokio dependency, clean-room reading of reference terminals, and the libghostty oracle.
- **Added:** `docs/projects/mux/RUSTY-TERM-HANDOFF.md`: state at handoff, the remaining work as an ordered roadmap (GUI frame path first, then the throughput floors, incremental snapshot, compact cell), open problems, the rules a successor must follow and how to validate changes.
- **Known limitations:** documentation only. Performance figures are local measurements, quoted with their ranges.
