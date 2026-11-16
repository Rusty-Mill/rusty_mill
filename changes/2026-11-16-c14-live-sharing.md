---
category: Added
changelog: Mux live sharing (C14): share a session with another client as watch-only grants, with share-session, unshare-session and a share-changed event.
---
## 2026-11-16 - Mux live sharing (C14)

- **Added:** on a shared server, `share-session` gives one connection a watch-only grant on every block of a session, now and later: it can attach and watch, never type without a full lease grant. `unshare-session`, the time limit (at most a day) or a disconnect detaches it at once. New wire behind `Features::SHARING` (bit 46): `ShareSession` 0x25, `UnshareSession` 0x26 and the `ShareChanged` event (kind 35, `share-changed` in rules).
- **Known limitations:** a share names a connection, not a principal, and is not kept across a server restart. `list` does not show who watches. Needs Nano's acceptance of the wire on the PR.
