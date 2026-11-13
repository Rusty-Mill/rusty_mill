---
category: Added
changelog: rusty-mux-server --shared serves its own user and the users an allowlist names, with input leases scoped to the principal that made each block.
---
## 2026-11-13 - rusty_mux shared mode

- **Added:** C26 shared mode (card #1459), Linux only. `rusty-mux-server --shared ALLOW <socket>` runs one server that also serves the users the allowlist file names (`uid:N` or `user:NAME`, one per line, no wildcard, root only when named). Its socket is open to every local user and the list decides; SIGHUP reads it again and closes only the connections it no longer names. Each guest has its own verified principal. The file has the login map's checks (a regular file of root's or the server user's that no one else can write).
- **Added:** on a shared server an open block's input lease goes only to a client of the principal that created its session, or to one that principal granted with the C77 grant messages. No new wire message or feature bit. On a shared server renaming or closing anything in a session (`rename-*`, `name-session`, `close-block`) is for the server's own account and the principal that created the session, the C33 rule from #1590 (`Mux::authorize_manage`); a session with no creator is the account's. A guest revoked before it sent `Hello` is closed when it registers.
- **Known limitations:** no UDP and no `--idle-exit` in shared mode yet; admitting a user gives it the server account's power, as a login-map entry does.
