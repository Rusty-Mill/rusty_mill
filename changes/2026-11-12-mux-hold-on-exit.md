---
category: Added
changelog: rusty_mux blocks can be held after their program exits, read-only with their last screen.
---
## 2026-11-12 - rusty_mux hold on exit

- **Added:** C78 (card #752). This sits behind `Features::HOLD` (bit 43), and a launch turns it on with `Launch.hold` (field 60).
  - When a held block's program exits, the block keeps its screen and scrollback as read-only and refuses input.
  - It stays until it is closed, and it survives a server restart.
- **Added:** block entries carry `exited` (field 61). `rusty-mux-ctl list` shows `state=exited` or `state=running`, and `terminal.shell` reports `activity exited` for a block whose program is gone.
- **Added:** `rusty-mux-ctl own` takes `"hold":true` on a launch.
