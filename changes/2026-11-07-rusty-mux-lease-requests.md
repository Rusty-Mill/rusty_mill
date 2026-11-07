---
category: Added
changelog: rusty_mux viewers can ask an agent block's owner for its input lease (Q26), so the GUI's Take over can get a grant.
---
## 2026-11-07 - rusty_mux: lease requests

- **Added:** `Features::LEASE_REQUESTS` (bit 37). A client sends `RequestLease` to ask the owner of an owner-only block for a grant. The server forwards it to the owner only, as `LeaseRequested` naming the client and its verified principal, and never as an event. The owner alone decides, with `GrantLease` or by not answering.
- **Added:** a request is refused for the owner itself, for a block anyone may type into, for an orphaned block, and while the owner is reconnecting or can't hear requests.
- **Added:** `rusty-mux-ctl own` prints `lease-requested` lines with the block, the client and its principal.
- **Known limitations:** the GUI's Take over that sends the request is separate work in the GUI.
