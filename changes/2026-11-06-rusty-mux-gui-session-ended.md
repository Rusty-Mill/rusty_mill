---
category: Added
changelog: rusty-mux-gui says when the session it showed ended while it was reconnecting, and offers the switcher to show another.
---
## 2026-11-06 - rusty-mux-gui: session ended panel

- **Added:** arc 2 slice S4d (`docs/projects/term/ARC2-MUX-CLIENT.md`, gui-design D28, D62, handoff D112, D114). When a reconnect finds the session the window showed gone, the window keeps running: the body shows "Session {name} ended" with an alert, what happened, and "Open switcher", which opens the session switcher (gui-design D60). The ribbon says "Connected to {target}" and "0 blocks"; the strip keeps the session's name and "+" is disabled. Picking another session shows it on the same connection.
- **Changed:** rusty_term's session-source port gains `Remote::ended`. The switcher starts on the first session it can show when none is current.
- **Known limitations:** a session that ends while the window is connected still closes the window, as before (handoff D114; whether the panel shows there too is open as handoff Q24).
