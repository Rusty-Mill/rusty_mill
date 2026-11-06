---
category: Added
changelog: rusty-mux-gui brings a lost connection back by itself, says so in a banner under the strip, and dims the blocks while it is away.
---
## 2026-11-06 - rusty-mux-gui: reconnecting, reconnected and error banners

- **Added:** arc 2 slice S4c (`docs/projects/term/ARC2-MUX-CLIENT.md`, gui-design D62, handoff D110-D113). When the control connection ends, the GUI takes its own client and its open blocks' clients over on a fresh connection, inside the server's 30 s grace, first at once and then after 2, 4, 8 and 16 s. The blocks keep their screens and their input lease.
- **Added:** the reconnecting banner: "Reconnecting to {target}, {n} s left", a bar that runs down with the grace, "Retry now" (tries at once, the count goes on) and "Stop trying". Once back, "Reconnected to {target}" with Dismiss.
- **Added:** the error banner, "Can't reach {target}", with a 2 px oxide top border and Retry, once the grace runs out, the user stops trying, or the server let the client go. Retry takes the clients over while the server may still keep them; otherwise it attaches anew, and the banner says the leases from before were released.
- **Added:** while the link is down the blocks show at 70% and take no clicks or keys, the strip's tabs, "+" and detached chip fade to 45% and take no clicks, the switcher's sessions can't be picked, and the ribbon's dot says "Reconnecting to" or "Not connected to". Caption buttons, the session chip and Settings stay live.
- **Changed:** `rusty_mux_client::attach` gains `take_over_with` (Hello with the caller's features, since the server only continues a client asking for exactly its features) and the shared `backoff`; the TUI uses the same schedule as before. rusty_term's session-source port gains `Source::retry`, `Source::stop`, `Link` and `Remote::link`. Pixmaps carry an opacity, so the CPU and GPU renderers can draw a veil; a veiled CPU frame is painted whole so the veil never stacks.
- **Known limitations:** the "Session {name} ended" panel is a follow-up (S4d, handoff D112); until then a session gone after a reconnect closes the window as before. A token the server refuses shows the same error banner as a grace that ran out (handoff D113). A block connection lost while the control connection stays up waits for the next control reconnect.
