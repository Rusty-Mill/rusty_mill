---
category: Added
changelog: rusty_mux blocks can be owned by the client that launches them (C46), and `rusty-mux-ctl own` holds that owner connection for an orchestrator.
---
## 2026-11-06 - rusty_mux: block ownership (C46)

- **Added:** `Features::OWNERSHIP` (bit 35). A `CreateBlock` or `Split` whose `Launch.own` is set makes the launching client the block's owner. Only that client may rename, close, split, resize, swap, move or detach it, and no other client may disconnect it. Attaching, watching, listing, zoom and focus stay open to everyone. Ownership belongs to the client, not its account, so a viewer of the same account gains nothing. A reconnect inside the 30 s takeover grace keeps it.
- **Added:** an owner gone past the grace leaves its block orphaned (pack D85): the block and its program stay, its lease ends, nobody may type into it or change it, and only a client of the owner's account may close it. `list` shows a block's `owner` (a client id, or `orphaned`).
- **Changed:** stopping the server only hangs up an owned block's program instead of killing it (pack D87).
- **Added:** `rusty-mux-ctl own`, one long-lived owner connection driven by JSON lines on stdin (`launch`, `close`), printing answers and the owned blocks' `program-exited`, `lease-changed` and `block-closed`, plus `reconnected` and `lost` (pack D118). It takes its client over after a dropped connection within the grace.
- **Known limitations:** the input gate (owner-only input, grants and the input stream to the owner, C77), signal and release (C76), passed descriptors and exact environment (C75) and hold on exit (C78) are separate rows. Ownership is not saved across a server restart.
