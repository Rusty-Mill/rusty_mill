---
category: Changed
changelog: The mux GUI shows how far a file drop or paste upload is and lets go of a server connection that stops accepting writes.
---
## 2026-11-26 - Mux GUI upload progress and write timeout

- **Changed:** a file dropped or pasted on a remote pane shows "Sending file: name: N%" in the banner while it uploads (cards #1629), and the banner clears when the path is typed. `Source::upload` takes a progress callback; no wire change.
- **Fixed:** writes to the server from a pane are bounded to 30 s, and a failed or timed-out write lets the connection go, so a stalled server can no longer hold the pane's writer lock, and typing with it, for ever (card #1631).
- **Known limitations:** no Cancel button; the timeout is covered by a unit test of the setting, not by a stalled-server test; Windows is not checked.
