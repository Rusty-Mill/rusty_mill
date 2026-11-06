---
category: Added
changelog: rusty_mux owner-only blocks take input only from clients their owner grants, and the owner hears every byte typed (C77).
---
## 2026-11-06 - rusty_mux: input gate (C77)

- **Added:** `Features::INPUT_GATE` (bit 36). An owned launch with `Launch.input` owner-only makes a block whose input lease only its owner, or a client the owner granted, may take; other clients get `input_gated`. Watching is unchanged. `list` shows a block's `input` (`open` or `owner-only`).
- **Added:** `GrantLease` and `RevokeLease`, owner only. A grant names one client and ends at a time at most 24 h ahead; revoking takes the lease back. `grant-changed` events tell clients when a grant starts or ends. A grant survives the owner's reconnect inside the 30 s grace.
- **Added:** `InputBatch`: every byte written to an owner-only block's program (`Input` and `terminal.send-keys`) is sent to the owner after it reaches the program, numbered per block. It is a message, never an event, so nobody else sees it and it is never replayed.
- **Added:** input fails closed: it is refused (`input_unrecorded`) while the owner is reconnecting or too far behind on its output to be told.
- **Added:** `rusty-mux-ctl own` takes `"input":"owner-only"` on `launch`, and `grant` and `revoke` requests, and prints `grant-changed` and `input` lines (bytes as hex).
- **Known limitations:** the GUI can't yet ask an owner for a grant, so its Take over still waits on that (pack Q26). Grants are not saved across a server restart.
