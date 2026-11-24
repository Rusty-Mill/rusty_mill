---
category: Added
changelog: Clicking a localhost link in a remote rusty-mux-gui pane forwards its port and opens it in the local browser.
---
## 2026-11-24 - GUI click-to-tunnel for loopback links (C81 slice 3, card #1584)

- **Added:** in a pane whose server forwards ports, a `localhost`, `127.0.0.1` or `[::1]` http(s) link asks once per pane and port, then tunnels the port over the pane's own connection and opens the link at a local port. `Source::forwards` and `Source::forward` carry it; `rusty_mux_client::forward::run_fed` runs a forward on a connection another reader owns.
- **Known limitations:** untested on a real display. A port nothing listens on shows as a dropped browser connection, not a notice. Trust lasts for the window's life. Without the input lease a notice says to take over.
