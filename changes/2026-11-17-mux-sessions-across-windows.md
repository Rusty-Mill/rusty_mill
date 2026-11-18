---
category: Added
changelog: rusty-mux-gui windows can open a session in a new window, move it to another window, or show it in every window of the same account (C64).
---
## 2026-11-17 - Sessions across windows (C64)

- **Added:** `rusty_mux` protocol `Features::SHOW_SESSION` (bit 51): `ShowSession` (0x30) asks one other client, or every other client, of the same account to show a session, and each hears `SessionShown` (0x92), a message and never an event, so a reconnect never repeats it. The server relays only, limits a client to 4 asks a second, and refuses another account's clients. `Entry::Client` gains `showable`.
- **Added:** in `rusty-mux-gui`, Right (or Tab) on a switcher row opens that session's menu: Open in new window, Move to a window, Show in every window. A window that is asked switches and shows an info banner, and waits while a take-over confirm or the palette is open.
- **Known limitations:** Move switches this window away as soon as it asks, so a refused ask is not reported back; no "…" button on the row yet (keyboard only), Move lists windows flat rather than in a submenu, and a window row does not say which session that window shows.
