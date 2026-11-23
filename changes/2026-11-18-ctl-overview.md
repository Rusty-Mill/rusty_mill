---
category: Added
changelog: rusty-mux-ctl overview lists blocks grouped by the card their agent works on, with each agent and role underneath.
---
## 2026-11-18 - rusty_mux: `rusty-mux-ctl overview` (#1572)

- **Added:** `rusty-mux-ctl overview` (`--json` for one object per card). Scoped to the session it runs in (outside any session, every session). One `List`, asked with `INSPECT` and `AGENT_META`; blocks are grouped by the card in their `Entry::Agent`, in order of first appearance, and blocks with no agent or card come last under "no card". Each block shows its state (running or exited), whether its lease is held, and its agent and role.
- **Not included:** no new wire, no server change. A server that does not grant `AGENT_META` lists every block under "no card". The GUI and TUI overview views are separate slices.
