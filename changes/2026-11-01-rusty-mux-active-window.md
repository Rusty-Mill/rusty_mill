---
category: Added
changelog: rusty_mux active window: each session has an active window on the server, set by `select-window` and announced by `active-window-changed`.
---
## 2026-11-01 - rusty_mux active window (C38)

- **Added:** each session has an active window, held by the server: its first window, until the new `select-window` request picks another (ctl `select-window --window W`, and the MCP tool of the same name). The session entry of `list` shows it as `active`.
- **Added:** `Event::ActiveWindowChanged` (kind 29) names the session and its new active window, none once it has no windows. The request and the event sit behind the new `Features::ACTIVE_WINDOW` (bit 24); a client without it never receives the event, live or on replay. `rusty-mux-ctl events`, `watch`, `wait --event` and `on` rules know it as `active-window-changed session=… window=…`.
- **Known limitations:** the attach client does not switch windows yet; it shows the window of the block it attached to. No request closes a window yet, so closing a window's last block leaves the (now empty) window active; the model hands the active window to the newest window left once windows can be closed.
