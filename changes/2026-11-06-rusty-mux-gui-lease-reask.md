---
category: Fixed
changelog: rusty-mux-gui takes a block's input back once another client lets it go, so typing into the focused block works again without moving the focus away and back.
---
## 2026-11-06 - rusty-mux-gui: ask again for a freed input lease

- **Fixed:** arc 2 slice S5 (`docs/projects/term/ARC2-MUX-CLIENT.md`, handoff D115, D116). When another client holds the focused block's input lease, the GUI's ask is refused as before. Once that client releases the lease, the GUI asks again, so typing reaches the block. It never takes a lease from a holder and never polls. When another client takes the lease, the GUI marks it as no longer held.
- **Not in this slice:** F3 "Take over" (gui-design D44) waits for block ownership and owner-granted leases (C46, C77). Nano chose that on 2026-11-06, because the server can't move a lease someone holds and an agent block can't be told apart yet. The TUI still asks only when the focus moves (handoff Q25).
