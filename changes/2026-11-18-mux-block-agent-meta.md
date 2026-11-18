---
category: Added
changelog: A block can be launched with an agent, role and card, which the mux server stores and lists back (Features bit 52); rusty-mux-ctl own takes them as launch keys.
---
## 2026-11-18 - rusty_mux: agent, role and card on blocks (D336)

- **Added:** `Features::AGENT_META` (bit 52). `Launch::agent: Option<AgentMeta>` (wire fields 72 agent, 73 role, 74 card; all three or none; each 1 to 128 bytes, no control characters) and `Entry::Agent { block, meta }`, listed after the block's `Entry::Block` to a client that asked. There is no `project` field: the session is the project.
- **Added:** the server stores the three texts on the block and echoes them; it never branches on them. `rusty-mux-ctl own` launch requests take `agent`, `role` and `card`; `rusty-mux-ctl` lists the `agent` entry.
- **Known limitations:** the texts are not kept across a server restart (C45 does not save them yet), and no event announces them: a client reads them from `List`. The GUI and TUI do not ask for the bit yet; the session overview (#1572) will. Wire change, so it passes the owner gate (D336).
