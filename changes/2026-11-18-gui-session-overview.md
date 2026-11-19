---
category: Added
changelog: rusty_mux_gui shows a session overview, the session's blocks as tiles in one row per card with agent, role and state, from the palette, the session menu or Ctrl-] o.
---
## 2026-11-18 - rusty_mux_gui: session overview (#1572, D97, D98)

- **Added:** a full-body view of the current session, one row per card (`No card` last, the grouping of `rusty-mux-ctl overview`), one tile per block with agent, role, state, "Exited"/"Detached" and who else has input. Opened from the palette ("Session overview"), the session menu, or `Ctrl-]` then `o`, which also closes it; arrows or the wheel move, Enter or a click opens the block (in another window, that window is shown and the block focused), Esc closes. The wheel, a dropped file and committed IME text never reach the panes under the view. The GUI now asks for `Features::AGENT_META`, and the action is hidden when the server lacks it. No wire change.
- **Known limitations:** no card title or state ribbon, live thumbnails or elapsed time (not on the wire: Q92). The GUI asks for a new List when a block is created, so a new agent's card shows at once.
